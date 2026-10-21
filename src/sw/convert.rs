//! SolidWorks assembly -> SolidCraft designs (structure and geometry). Port of `sw/convert.py`.
//!
//! Every part becomes its own .solidcraft design (an Import feature holding the part's exact
//! geometry), and the assembly becomes a design whose components link to those part designs,
//! one occurrence per SolidWorks instance, placed where SolidWorks places it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::assembly::{Assembly, Instance, read_assembly};
use super::container::read_file;
use super::ir;
use crate::converter::{Hooks, Stage};
use crate::mesh::Mesh;
use crate::report::{PartResult, Report};
use crate::solidcraft::assembly::add_moves;
use crate::solidcraft::script::{self as sc, METADATA_KEY, Measured, Script, is_solidcraft_error};
use crate::solidcraft::transform::{Matrix, Vec3, Y_UP_TO_Z_UP, apply, conjugate, transpose};
use crate::step::fix as step_fix;
use crate::step::mesh_fix::{self, REPAIRS};
use crate::util::{self, py_repr, read_latin1, round1, s, write_latin1};

/// SolidWorks stores a placement for row vectors (p' = p R + T); SolidCraft rotates column
/// vectors, so the stored matrix is used transposed.
const ROW_VECTOR_CONVENTION: bool = true;

pub const VOLUME_TOLERANCE: f64 = 0.005; // relative
pub const BOX_TOLERANCE: f64 = 0.1; // mm

pub const PREVIEW_SOURCE: &str = "preview_solidworks.png";
pub const PREVIEW_RESULT: &str = "preview_solidcraft.png";

/// cadmpeg: the one shipped beside this program first, then the patched build
/// (packaging/cadmpeg/bin), then an installed one.
pub fn find_cadmpeg(explicit: Option<&str>) -> Result<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    c.extend(explicit.map(PathBuf::from));
    c.extend(std::env::var_os("CADMPEG").map(PathBuf::from));
    if let Some(dir) = util::exe_dir() {
        c.push(dir.join("cadmpeg.exe"));
    }
    c.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/packaging/cadmpeg/bin/cadmpeg.exe")));
    c.extend(util::which("cadmpeg"));
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        c.push(PathBuf::from(home).join(".cargo").join("bin").join("cadmpeg.exe"));
    }
    c.into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| anyhow!("cadmpeg not found: install it with `cargo install cadmpeg`"))
}

/// Find the file an instance refers to, as SolidWorks does: by its file name.
pub fn resolve_reference(asm: &Assembly, stem: &str, search: &[PathBuf]) -> Option<PathBuf> {
    let wanted = [format!("{stem}.sldprt").to_lowercase(), format!("{stem}.sldasm").to_lowercase()];
    let matches = |p: &Path| p.file_name().is_some_and(|n| wanted.contains(&n.to_string_lossy().to_lowercase()));
    for folder in search {
        if let Ok(rd) = std::fs::read_dir(folder) {
            for e in rd.flatten() {
                if matches(&e.path()) {
                    return Some(e.path());
                }
            }
        }
    }
    for r in &asm.references {
        // the absolute paths the assembly recorded
        let p = PathBuf::from(r);
        if matches(&p) && p.is_file() {
            return Some(p);
        }
    }
    for folder in search {
        // last resort: anywhere below the search folders
        let mut todo = vec![folder.clone()];
        while let Some(dir) = todo.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut subdirs = Vec::new();
            for e in rd.flatten() {
                let p = e.path();
                if matches(&p) {
                    return Some(p);
                }
                if p.is_dir() {
                    subdirs.push(p);
                }
            }
            todo.extend(subdirs.into_iter().rev());
        }
    }
    None
}

fn run_cadmpeg(cadmpeg: &Path, src: &Path, step: &Path) -> Result<Vec<String>> {
    let out = util::run(cadmpeg, &["convert", &s(src), "-o", &s(step), "--force"], Duration::from_secs(900))?;
    let all = format!("{}{}", out.stdout, out.stderr);
    let notes: Vec<&str> = all.lines().filter(|l| !l.trim().is_empty()).collect();
    if out.status != Some(0) || !step.is_file() {
        return Err(anyhow!(
            notes.last().map(|s| s.to_string()).unwrap_or_else(|| format!("cadmpeg exited with {}", out.status.unwrap_or(-1)))
        ));
    }
    // Keep what affects the exact geometry; display tessellation and design-intent notes
    // (sketch relations, used later for the timeline) are not problems of this phase.
    let mut keep = Vec::new();
    for n in notes {
        let low = n.to_lowercase();
        if low.contains("[warning/geometry]") && !low.contains("tessellation(s) require") {
            keep.push(n.trim().to_owned());
        } else if low.starts_with("check:") && !low.contains("0 error(s)") {
            keep.push(n.trim().to_owned());
        }
    }
    Ok(keep)
}

/// Text as the inside of a STEP (ISO 10303-21) string literal.
pub fn step_string(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            c if (32..127).contains(&(c as u32)) => out.push(c),
            c => {
                let mut units = [0u16; 2];
                let hex: String = c.encode_utf16(&mut units).iter().map(|u| format!("{u:04X}")).collect();
                out.push_str(&format!("\\X2\\{hex}\\X0\\"));
            }
        }
    }
    out
}

/// Give the STEP's product (and so the imported body) the part's name.
pub fn name_step_product(step: &Path, name: &str) -> Result<()> {
    let text = read_latin1(step)?;
    let named = step_string(name);
    let text = text.replacen("PRODUCT('cadmpeg_model','cadmpeg_model'", &format!("PRODUCT('{named}','{named}'"), 1);
    let text = text.replacen("FILE_NAME('cadmpeg_model'", &format!("FILE_NAME('{named}'"), 1);
    write_latin1(step, &text)
}

fn box_error(measured_box: ([f64; 3], [f64; 3]), truth: ([f64; 3], [f64; 3])) -> f64 {
    let ((blo, bhi), (lo, hi)) = (measured_box, truth);
    (0..3).map(|k| (blo[k] - lo[k]).abs().max((bhi[k] - hi[k]).abs())).fold(f64::NEG_INFINITY, f64::max)
}

/// (relative volume error, box error in mm, share of volume inside out) against the mesh.
fn mismatch(measured: &Value, truth: &Mesh) -> Result<(f64, f64, f64)> {
    let m = Measured(measured);
    let vol = m.total_volume();
    let bbox = m.bbox()?;
    let (Some(vol), Some(bbox)) = (vol, bbox) else { return Ok((f64::INFINITY, f64::INFINITY, 0.0)) };
    let tv = truth.volume();
    let tb = truth.bbox().ok_or_else(|| anyhow!("min() arg is an empty sequence"))?;
    let bx = box_error(bbox, tb);
    let neg = m.negative_volume();
    Ok(((vol - tv).abs() / tv.abs().max(1e-9), bx, neg / tv.abs().max(1e-9)))
}

/// Repair the orientation flags, trying each way until one matches SolidWorks' mesh.
fn repair_step(step: &Path, src: &Path, build: &Path, cadmpeg: &Path, cli: &Path, res: &mut PartResult) -> Result<()> {
    let truth = ir::part_mesh(src, cadmpeg, build).ok();
    let raw = read_latin1(step)?;
    let Some(truth) = truth.filter(|t| !t.triangles.is_empty()) else {
        step_fix::repair_file(step)?; // no reference: rules only
        res.repair = Some("reglas (sin malla de referencia)".into());
        return Ok(());
    };
    let trial = build.join(format!("{}.trial.step", util::stem(step)));
    let mut best: Option<(f64, &str, String)> = None;
    for (name, opts) in REPAIRS {
        let (text, _fix) = mesh_fix::repair_text(&raw, &truth, opts)?;
        write_latin1(&trial, &text)?;
        let measured = match sc::evaluate(cli, &trial) {
            Ok(m) => m,
            Err(e) if is_solidcraft_error(&e) => continue,
            Err(e) => return Err(e),
        };
        let (vol_err, box_err, neg) = mismatch(&measured, &truth)?;
        let score = vol_err + box_err / 100.0 + 2.0 * neg;
        if best.as_ref().is_none_or(|b| score < b.0) {
            best = Some((score, name, text));
        }
        if vol_err <= VOLUME_TOLERANCE / 5.0 && box_err <= BOX_TOLERANCE / 2.0 && neg < 1e-4 {
            break; // good enough: no need to try the others
        }
    }
    if trial.exists() {
        std::fs::remove_file(&trial)?;
    }
    match best {
        None => {
            step_fix::repair_file(step)?;
            res.repair = Some("reglas".into());
        }
        Some((_, name, text)) => {
            write_latin1(step, &text)?;
            res.repair = Some(name.to_owned());
        }
    }
    Ok(())
}

/// Compare the converted part with SolidWorks' own display mesh of it.
fn check_against_source(src: &Path, build: &Path, cadmpeg: &Path, measured: &Value, res: &mut PartResult, up_y: bool) -> Result<()> {
    let truth = match ir::part_mesh(src, cadmpeg, build) {
        Ok(t) => t,
        Err(e) => {
            res.warnings.push(format!("no se ha podido comparar con SolidWorks: {e}"));
            return Ok(());
        }
    };
    if truth.triangles.is_empty() {
        return Ok(());
    }
    let truth = if up_y { truth.transformed(&Y_UP_TO_Z_UP) } else { truth };
    let source_volume = truth.volume();
    res.source_volume_mm3 = Some(source_volume);
    let bbox = Measured(measured).bbox()?;
    let (Some(bbox), Some(volume)) = (bbox, res.volume_mm3) else { return Ok(()) };
    let tb = truth.bbox().ok_or_else(|| anyhow!("min() arg is an empty sequence"))?;
    let box_err = box_error(bbox, tb);
    let vol_err = (volume - source_volume).abs() / source_volume.abs().max(1e-9);
    let matches = vol_err <= VOLUME_TOLERANCE && box_err <= BOX_TOLERANCE;
    res.matches_source = Some(matches);
    if !matches {
        res.warnings.push(format!(
            "no coincide con SolidWorks: volumen {:.1} cm³ frente a {:.1} cm³, caja desviada {box_err:.2} mm",
            volume / 1000.0,
            source_volume / 1000.0
        ));
    }
    Ok(())
}

/// The measurements a part result takes from `solidcraft-cli eval`.
pub fn take_measurements(res: &mut PartResult, measured: &Value) {
    let m = Measured(measured);
    res.bodies = m.body_count();
    res.volume_mm3 = m.total_volume();
    res.ok = res.bodies.is_some_and(|b| b != 0);
}

pub fn inside_out_warnings(res: &mut PartResult, measured: &Value) {
    for body in Measured(measured).bodies() {
        if body.get("volume_mm3").and_then(Value::as_f64).unwrap_or(0.0) < 0.0 {
            let name = match body.get("name") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Null) | None => "None".into(),
                Some(other) => other.to_string(),
            };
            res.warnings.push(format!("el sólido {name} está del revés (volumen negativo)"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn convert_part(
    src: &Path,
    stem: &str,
    out: &Path,
    cli: &Path,
    cadmpeg: &Path,
    log: &dyn Fn(&str),
    up_y: bool,
    design_dir: Option<&Path>,
) -> PartResult {
    let t0 = Instant::now();
    let mut res = PartResult::new(stem, Some(s(src)));
    let build = out.join("build");
    let step = build.join(format!("{stem}.step"));
    let design = design_dir.map(Path::to_path_buf).unwrap_or_else(|| out.join("parts")).join(format!("{stem}.solidcraft"));
    let attempt = (|| -> Result<()> {
        if let Some(dir) = design.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::create_dir_all(&build)?;
        res.warnings.extend(run_cadmpeg(cadmpeg, src, &step)?);
        name_step_product(&step, stem)?;
        repair_step(&step, src, &build, cadmpeg, cli, &mut res)?;
        if up_y {
            // Turned here rather than with insert_step's up: "y": SolidCraft 0.16.2 rebuilds a
            // turned import differently once the design is saved and opened again.
            write_latin1(&step, &step_fix::rotate_text(&read_latin1(&step)?, &Y_UP_TO_Z_UP)?)?;
        }
        res.step = Some(s(&step));
        let mut script = Script::default();
        script.add("file.new", json!({ "name": stem }));
        script.add("file.insert_step", json!({ "path": s(&step) }));
        script.add(
            "document.metadata.set",
            json!({"key": METADATA_KEY, "value": {"source": s(src), "source_cad": "SolidWorks"}}),
        );
        script.add("file.save_as", json!({ "path": s(&design) }));
        sc::run(cli, &script.write(&build.join(format!("{stem}.part.json")))?)?;
        let measured = sc::evaluate(cli, &design)?;
        res.design = Some(s(&design));
        take_measurements(&mut res, &measured);
        if !res.ok {
            res.warnings.push("la pieza convertida no tiene sólidos".into());
        }
        inside_out_warnings(&mut res, &measured);
        check_against_source(src, &build, cadmpeg, &measured, &mut res, up_y)?;
        Ok(())
    })();
    if let Err(e) = attempt {
        // one bad part must not stop the assembly
        res.warnings.push(e.to_string());
    }
    res.seconds = round1(t0.elapsed().as_secs_f64());
    log(&res.log_line());
    res
}

/// Rotation (for column vectors) and translation (mm) of an occurrence in SolidCraft.
///
/// With `up_y` every part design already holds its bodies turned upright (U), so an occurrence
/// placed at M, t in SolidWorks becomes U·M·Uᵀ, U·t.
pub fn placement(inst: &Instance, up_y: bool) -> (Matrix, Vec3) {
    let mut m = if ROW_VECTOR_CONVENTION { transpose(&inst.rotation) } else { inst.rotation };
    let mut t = inst.translation_mm;
    if up_y {
        m = conjugate(&Y_UP_TO_Z_UP, &m);
        t = apply(&Y_UP_TO_Z_UP, t);
    }
    (m, t)
}

pub fn assembly_script(
    asm: &Assembly,
    part_designs: &IndexMap<String, PathBuf>,
    out_design: &Path,
    up_y: bool,
) -> (Script, usize, IndexMap<String, String>) {
    let mut sc = Script::default();
    sc.add("file.new", json!({ "name": asm.name }));
    let mut counts: IndexMap<&str, usize> = IndexMap::new();
    let mut mapping: IndexMap<String, String> = IndexMap::new();
    let mut placed = 0;
    // Every occurrence first: a copy takes the current placement of the occurrence it copies,
    // so copies are made while the first occurrence is still at the origin.
    let mut todo: Vec<(&Instance, String)> = Vec::new();
    for inst in &asm.instances {
        let Some(design) = part_designs.get(&inst.stem) else { continue };
        let n = counts.get(inst.stem.as_str()).copied().unwrap_or(0) + 1;
        counts.insert(&inst.stem, n);
        if n == 1 {
            sc.add("component.insert_design", json!({"path": s(design), "name": inst.stem}));
        } else {
            sc.add("occurrence.copy", json!({ "occurrence": format!("{}:1", inst.stem) }));
        }
        let occ = format!("{}:{n}", inst.stem);
        mapping.insert(inst.name.clone(), occ.clone());
        todo.push((inst, occ));
    }
    for (inst, occ) in &todo {
        let (m, t) = placement(inst, up_y);
        add_moves(&mut sc, occ, &m, &t);
        placed += 1;
    }
    let meta = json!({"source": s(&asm.path), "source_cad": "SolidWorks", "instances": mapping, "up": if up_y { "y" } else { "z" }});
    sc.add("document.metadata.set", json!({"key": METADATA_KEY, "value": meta}));
    sc.add("file.save_as", json!({ "path": s(out_design) }));
    (sc, placed, mapping)
}

/// The preview picture SolidWorks saved inside a part or assembly file, as PNG bytes.
pub fn preview_png(src: &Path) -> Option<Vec<u8>> {
    let blocks = read_file(src).ok()?;
    blocks
        .into_iter()
        .rev()
        .find(|b| b.name == "PreviewPNG" && b.data.as_ref().is_some_and(|d| d.starts_with(b"\x89PNG")))
        .and_then(|b| b.data)
}

fn save_preview(src: &Path, png: &Path) -> Result<()> {
    if let Some(data) = preview_png(src) {
        std::fs::write(png, data)?;
    }
    Ok(())
}

pub struct Tools {
    pub cli: PathBuf,
    pub cadmpeg: PathBuf,
}

/// The result picture and report every conversion ends with.
pub fn finish(report: &mut Report, out: &Path, cli: &Path, picture: bool, hooks: &Hooks, t0: Instant, done: usize) -> Result<()> {
    if picture
        && let Some(output) = report.output.clone() {
            hooks.tell(Stage::Picture, done, done, None);
            hooks.log("Dibujando una imagen del resultado…");
            if let Err(e) = sc::snapshot(cli, Path::new(&output), &out.join(PREVIEW_RESULT), "iso", 800, 600) {
                report.warnings.push(format!("imagen: {e}"));
            }
        }
    report.seconds = round1(t0.elapsed().as_secs_f64());
    util::write_json(&out.join("report.json"), report)?;
    hooks.log(&report.finished_line());
    Ok(())
}

pub fn convert_assembly(src: &Path, out_root: &Path, up_y: bool, tools: &Tools, picture: bool, hooks: &Hooks) -> Result<Report> {
    let t0 = Instant::now();
    let asm = read_assembly(src)?;
    let out = out_root.join(&asm.name);
    std::fs::create_dir_all(&out)?;
    let mut report = Report::new(s(src));
    report.instances = asm.instances.len();
    report.warnings = asm.warnings.clone();
    hooks.log(&format!("{}: {} instancias", asm.name, asm.instances.len()));
    let mut stems: Vec<String> = Vec::new();
    for i in &asm.instances {
        if !stems.contains(&i.stem) {
            stems.push(i.stem.clone());
        }
    }
    hooks.tell(Stage::Start, 0, stems.len(), None);

    let mut part_designs: IndexMap<String, PathBuf> = IndexMap::new();
    let folder = src.parent().map(Path::to_path_buf).unwrap_or_default();
    for (n, stem) in stems.iter().enumerate() {
        let r = match resolve_reference(&asm, stem, std::slice::from_ref(&folder)) {
            None => {
                hooks.log(&format!("  ERR {stem}: no se encuentra el fichero"));
                PartResult::with_warning(stem, None, "no se encuentra el fichero")
            }
            Some(r) if util::suffix_lower(&r) == ".sldasm" => {
                hooks.log(&format!("  ERR {stem}: los subensamblajes aún no se convierten"));
                PartResult::with_warning(stem, Some(s(&r)), "los subensamblajes aún no se convierten")
            }
            Some(r) => {
                let r = convert_part(&r, stem, &out, &tools.cli, &tools.cadmpeg, &|l| hooks.log(l), up_y, None);
                if r.ok
                    && let Some(d) = &r.design {
                        part_designs.insert(stem.clone(), PathBuf::from(d));
                    }
                r
            }
        };
        hooks.tell(Stage::Part, n + 1, stems.len(), Some(&r));
        report.parts.push(r);
    }
    hooks.tell(Stage::Assembly, stems.len(), stems.len(), None);

    let design = out.join(format!("{}.solidcraft", asm.name));
    let (script, placed, _mapping) = assembly_script(&asm, &part_designs, &design, up_y);
    let built = script.write(&out.join("build").join(format!("{}.assembly.json", asm.name))).and_then(|p| sc::run(&tools.cli, &p));
    match built {
        Ok(_) => {
            report.output = Some(s(&design));
            report.placed = placed;
        }
        Err(e) => report.warnings.push(format!("ensamblaje: {e}")),
    }
    save_preview(src, &out.join(PREVIEW_SOURCE))?;
    finish(&mut report, &out, &tools.cli, picture, hooks, t0, stems.len())?;
    Ok(report)
}

/// Convert one part (.SLDPRT) into its own SolidCraft design.
pub fn convert_single_part(src: &Path, out_root: &Path, up_y: bool, tools: &Tools, picture: bool, hooks: &Hooks) -> Result<Report> {
    let t0 = Instant::now();
    let stem = util::stem(src);
    let out = out_root.join(&stem);
    std::fs::create_dir_all(&out)?;
    let mut report = Report::new(s(src));
    report.instances = 1;
    hooks.log(&format!("{stem}: pieza suelta"));
    hooks.tell(Stage::Start, 0, 1, None);
    let r = convert_part(src, &stem, &out, &tools.cli, &tools.cadmpeg, &|l| hooks.log(l), up_y, Some(&out));
    hooks.tell(Stage::Part, 1, 1, Some(&r));
    if r.ok
        && let Some(d) = &r.design {
            report.output = Some(d.clone());
            report.placed = 1;
        }
    report.parts.push(r);
    save_preview(src, &out.join(PREVIEW_SOURCE))?;
    finish(&mut report, &out, &tools.cli, picture, hooks, t0, 1)?;
    Ok(report)
}

/// Placement metadata for the logs, as Python prints a float.
pub fn fmt_mm(v: f64) -> String {
    py_repr(v)
}

pub fn metadata(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
}

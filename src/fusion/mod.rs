//! Fusion design (.f3d / .f3z) -> SolidCraft designs.
//!
//! The converter reads Fusion files directly ([`offline`], no Fusion needed). This module keeps
//! the preview reader and the older route (port of `fusion/convert.py`), kept as in Python only to
//! compare results: Fusion itself exports the design (see [`bridge`]): one STEP per component with bodies, and
//! the placement of every occurrence, external references included. Each component becomes a
//! part design; the assembly links them, one occurrence per Fusion occurrence. Each part is
//! checked against the exact volumes and boxes Fusion reports.

pub mod bridge;
pub mod f3d;
pub mod offline;

use std::collections::HashSet;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Instant;

use anyhow::Result;
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::converter::{Hooks, Stage};
use crate::report::{PartResult, Report};
use crate::solidcraft::assembly::{Placement, assembly_script};
use crate::solidcraft::script::{self as sc, METADATA_KEY, Measured, Script, truthy};
use crate::solidcraft::transform::{IDENTITY, Matrix, Vec3, Y_UP_TO_Z_UP, apply, conjugate};
use crate::step::fix::rotate_text;
use crate::sw::convert::{BOX_TOLERANCE, VOLUME_TOLERANCE, finish, take_measurements};
use crate::util::{self, read_latin1, round1, s, write_latin1};

pub const PREVIEW_FUSION: &str = "preview_fusion.png";
const MAX_PREVIEW: u64 = 20 * 1024 * 1024;

fn read_entry<R: Read + std::io::Seek>(z: &mut zip::ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    let mut f = z.by_name(name).ok()?;
    let mut v = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut v).ok()?;
    Some(v)
}

fn best_preview<R: Read + std::io::Seek>(z: &mut zip::ZipArchive<R>) -> Option<Vec<u8>> {
    let mut best: Option<(u64, String)> = None;
    for i in 0..z.len() {
        let f = z.by_index_raw(i).ok()?;
        let name = f.name().ok()?.into_owned();
        let low = name.to_lowercase();
        if low.ends_with(".png") && low.contains("preview") && f.size() < MAX_PREVIEW && best.as_ref().is_none_or(|b| f.size() > b.0) {
            best = Some((f.size(), name));
        }
    }
    let data = read_entry(z, &best?.1)?;
    data.starts_with(b"\x89PNG").then_some(data)
}

/// The preview picture Fusion keeps inside a .f3d (or the root design of a .f3z).
pub fn preview_png(src: &Path) -> Option<Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(src).ok()?).ok()?;
    let names: Vec<String> = z.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
    if names.iter().any(|n| n == "Manifest.json") && !names.iter().any(|n| n.starts_with("FusionAssetName")) {
        let manifest: Value = serde_json::from_slice(&read_entry(&mut z, "Manifest.json")?).ok()?;
        let root = manifest.get("root")?.as_str()?.to_owned();
        if z.by_name(&root).ok()?.size() > 512 * 1024 * 1024 {
            return None;
        }
        let inner = read_entry(&mut z, &root)?;
        let mut inner = zip::ZipArchive::new(Cursor::new(inner)).ok()?;
        return best_preview(&mut inner);
    }
    best_preview(&mut z)
}

static BAD_CHARS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[<>:"/\\|?*\x00-\x1f]+"#).expect("name pattern"));

/// A file name for a component, unique among `used` (compared ignoring case).
pub fn stem_for(name: &str, used: &mut HashSet<String>) -> String {
    let text: String = name.nfkc().collect();
    let text = BAD_CHARS.replace_all(&text, "_");
    let text = text.trim_matches(|c| c == ' ' || c == '.');
    let text = if text.is_empty() { "Componente" } else { text };
    let (mut stem, mut n) = (text.to_owned(), 2);
    while used.contains(&stem.to_lowercase()) {
        stem = format!("{text} ({n})");
        n += 1;
    }
    used.insert(stem.to_lowercase());
    stem
}

fn turned_box(bbox: Option<&Value>, m: Option<&Matrix>) -> Option<(Vec3, Vec3)> {
    let b = bbox.filter(|b| truthy(b))?.as_array()?;
    let v3 = |v: &Value| -> Option<Vec3> {
        let a = v.as_array()?;
        Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
    };
    let (lo, hi) = (v3(b.first()?)?, v3(b.get(1)?)?);
    let Some(m) = m else { return Some((lo, hi)) };
    let mut pts = Vec::new();
    for x in [lo[0], hi[0]] {
        for y in [lo[1], hi[1]] {
            for z in [lo[2], hi[2]] {
                pts.push(apply(m, [x, y, z]));
            }
        }
    }
    let mut a = pts[0];
    let mut c = pts[0];
    for p in &pts[1..] {
        for k in 0..3 {
            a[k] = a[k].min(p[k]);
            c[k] = c[k].max(p[k]);
        }
    }
    Some((a, c))
}

fn id_of(c: &Value) -> String {
    match c.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => String::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn convert_component(c: &Value, job: &Path, out: &Path, stem: &str, cli: &Path, turn: bool, src: &Path, log: &dyn Fn(&str)) -> PartResult {
    let t0 = Instant::now();
    let mut res = PartResult::new(stem, Some(s(src)));
    let build = out.join("build");
    let design = out.join("parts").join(format!("{stem}.solidcraft"));
    let step = build.join(format!("{stem}.step"));
    let attempt = (|| -> Result<()> {
        std::fs::create_dir_all(&build)?;
        std::fs::create_dir_all(out.join("parts"))?;
        let from = c.get("step").and_then(Value::as_str).unwrap_or_default();
        std::fs::copy(job.join(from), &step)?;
        if turn {
            write_latin1(&step, &rotate_text(&read_latin1(&step)?, &Y_UP_TO_Z_UP)?)?;
        }
        res.step = Some(s(&step));
        let mut script = Script::default();
        script.add("file.new", json!({ "name": stem }));
        script.add("file.insert_step", json!({ "path": s(&step) }));
        script.add(
            "document.metadata.set",
            json!({"key": METADATA_KEY, "value": {"source": s(src), "source_cad": "Fusion", "component": c.get("name")}}),
        );
        script.add("file.save_as", json!({ "path": s(&design) }));
        sc::run(cli, &script.write(&build.join(format!("{stem}.part.json")))?)?;
        let measured = sc::evaluate(cli, &design)?;
        res.design = Some(s(&design));
        take_measurements(&mut res, &measured);
        res.repair = Some("STEP de Fusion".into());
        res.source_volume_mm3 = c.get("volume_mm3").and_then(Value::as_f64);
        let truth_box = turned_box(c.get("bbox_mm"), turn.then_some(&Y_UP_TO_Z_UP));
        let got = Measured(&measured).bbox()?;
        if let (Some(volume), Some(source), Some((lo, hi)), Some((blo, bhi))) = (res.volume_mm3, res.source_volume_mm3, truth_box, got)
            && source != 0.0 {
                let box_err = (0..3).map(|k| (blo[k] - lo[k]).abs().max((bhi[k] - hi[k]).abs())).fold(f64::NEG_INFINITY, f64::max);
                let vol_err = (volume - source).abs() / source.abs().max(1e-9);
                let matches = vol_err <= VOLUME_TOLERANCE && box_err <= BOX_TOLERANCE;
                res.matches_source = Some(matches);
                if !matches {
                    res.warnings.push(format!(
                        "no coincide con Fusion: volumen {:.2} cm³ frente a {:.2} cm³, caja desviada {box_err:.2} mm",
                        volume / 1000.0,
                        source / 1000.0
                    ));
                }
            }
        if !res.ok {
            res.warnings.push("la pieza convertida no tiene sólidos".into());
        }
        Ok(())
    })();
    if let Err(e) = attempt {
        res.warnings.push(e.to_string());
    }
    res.seconds = round1(t0.elapsed().as_secs_f64());
    log(&res.log_line());
    res
}

fn matrix3(rows: Option<&Value>) -> Option<Matrix> {
    let rows = rows?.as_array()?;
    let mut m = [0.0; 9];
    for r in 0..3 {
        let row = rows.get(r)?.as_array()?;
        for c in 0..3 {
            m[3 * r + c] = row.get(c)?.as_f64()?;
        }
    }
    Some(m)
}

fn vec3(v: Option<&Value>) -> Option<Vec3> {
    let a = v?.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}

pub fn convert_fusion(src: &Path, out_root: &Path, up_y: bool, cli: &Path, picture: bool, hooks: &Hooks) -> Result<Report> {
    let t0 = Instant::now();
    let name = util::stem(src);
    let out = out_root.join(&name);
    std::fs::create_dir_all(&out)?;
    let mut report = Report::new(s(src));
    hooks.log(&format!("{name}: diseño de Fusion"));
    hooks.tell(Stage::Export, 0, 0, None);
    let (data, job) = bridge::export(src, &|l| hooks.log(l))?;
    let turn = up_y && data.get("up").and_then(Value::as_str).unwrap_or("y") == "y";
    let empty = Vec::new();
    let comps: Vec<&Value> = data
        .get("components")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .filter(|c| c.get("step").is_some_and(truthy))
        .collect();
    let mut used = HashSet::new();
    let mut stems: IndexMap<String, String> = IndexMap::new();
    for c in &comps {
        let n = c.get("name").filter(|n| truthy(n)).and_then(Value::as_str).unwrap_or("Componente");
        stems.insert(id_of(c), stem_for(n, &mut used));
    }
    hooks.tell(Stage::Start, 0, comps.len(), None);
    let mut designs: IndexMap<String, PathBuf> = IndexMap::new();
    for (n, c) in comps.iter().enumerate() {
        let id = id_of(c);
        let r = convert_component(c, &job, &out, &stems[&id], cli, turn, src, &|l| hooks.log(l));
        if r.ok
            && let Some(d) = &r.design {
                designs.insert(id.clone(), PathBuf::from(d));
            }
        hooks.tell(Stage::Part, n + 1, comps.len(), Some(&r));
        report.parts.push(r);
    }

    let mut placements: Vec<Placement> = Vec::new();
    if let Some(root) = designs.get("root") {
        let label = data.get("name").and_then(Value::as_str).unwrap_or("root").to_owned();
        placements.push(Placement { stem: stems["root"].clone(), design: root.clone(), rotation: IDENTITY, translation_mm: [0.0; 3], label });
    }
    let occurrences = data.get("occurrences").and_then(Value::as_array).unwrap_or(&empty);
    for o in occurrences {
        let cid = match o.get("component") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => continue,
        };
        let Some(design) = designs.get(&cid) else { continue };
        let (Some(mut m), Some(mut t)) = (matrix3(o.get("rotation")), vec3(o.get("translation_mm"))) else { continue };
        if turn {
            m = conjugate(&Y_UP_TO_Z_UP, &m);
            t = apply(&Y_UP_TO_Z_UP, t);
        }
        let label = o.get("path").and_then(Value::as_str).unwrap_or("").to_owned();
        placements.push(Placement { stem: stems[&cid].clone(), design: design.clone(), rotation: m, translation_mm: t, label });
    }
    report.instances = placements.len();
    hooks.tell(Stage::Assembly, comps.len(), comps.len(), None);

    if placements.len() == 1 && Some(&placements[0].stem) == stems.get("root") && occurrences.is_empty() {
        report.output = Some(s(&placements[0].design)); // a single part
        report.placed = 1;
    } else if !placements.is_empty() {
        let design = out.join(format!("{name}.solidcraft"));
        let mut meta = Map::new();
        meta.insert("source".into(), json!(s(src)));
        meta.insert("source_cad".into(), json!("Fusion"));
        meta.insert("up".into(), json!(if turn { "y" } else { "z" }));
        let (script, _) = assembly_script(&name, &placements, &design, meta);
        match script.write(&out.join("build").join(format!("{name}.assembly.json"))).and_then(|p| sc::run(cli, &p)) {
            Ok(_) => {
                report.output = Some(s(&design));
                report.placed = placements.len();
            }
            Err(e) => report.warnings.push(format!("ensamblaje: {e}")),
        }
    } else {
        report.warnings.push("el diseño no tiene sólidos".into());
    }

    if let Some(png) = preview_png(src) {
        std::fs::write(out.join(PREVIEW_FUSION), png)?;
    }
    finish(&mut report, &out, cli, picture, hooks, t0, comps.len())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_names_become_unique_file_names() {
        let mut used = HashSet::new();
        assert_eq!(stem_for("Tapa: \"A\"", &mut used), "Tapa_ _A_");
        assert_eq!(stem_for("Eje", &mut used), "Eje");
        assert_eq!(stem_for("eje", &mut used), "eje (2)");
        assert_eq!(stem_for(" .. ", &mut used), "Componente");
        assert_eq!(stem_for("ﬁn", &mut used), "fin"); // NFKC
    }
}

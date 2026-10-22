//! Fusion design (.f3d / .f3z) -> SolidCraft designs, without Fusion installed. Port of
//! `fusion/offline.py`.
//!
//! * Structure: our own reader of the Design segment ([`super::f3d`]): components, every
//!   occurrence placement with its parent, which bodies each component owns, and each body's
//!   cached box.
//! * Geometry: cadmpeg converts each document's bodies to STEP. It writes them in a fixed order
//!   (by BREP blob, then by body within the blob), which tells which component each one belongs
//!   to. The file is split per component and tidied the way Fusion's own export writes STEP
//!   ([`crate::step::clean`]).
//! * External references (.f3z): a component whose name is the root component of another
//!   document in the archive is that document, placed there. Its geometry and inner placements
//!   come from that document (the copies the referencing design keeps are not grouped by
//!   component).
//!
//! Each part is checked against the boxes Fusion cached for its bodies.
//!
//! A component whose timeline can be made again (see [`super::rebuild`]) gets it: the part is
//! built from its features, its bodies are joined and named as Fusion's are, and it replaces the
//! imported part when both have the same bodies (names, boxes and volumes).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value, json};

use super::f3d::{Assembly, F3dError, IDENTITY4, Mat4, matmul4, open_f3d, read_assembly};
use super::rebuild::rebuild;
use super::sketch::{self, Sketch};
use super::timeline::Feature;
use super::{PREVIEW_FUSION, preview_png, stem_for};
use crate::converter::{Hooks, Stage};
use crate::report::{PartResult, Report};
use crate::solidcraft::assembly::{Placement, assembly_script, assembly_script_with, link_parts};
use crate::solidcraft::design::edit_document;
use crate::solidcraft::script::{self as sc, METADATA_KEY, Measured, Script};
use crate::solidcraft::transform::IDENTITY;
use crate::step::Step;
use crate::step::clean::clean;
use crate::step::split::StepFile;
use crate::sw::convert::{BOX_TOLERANCE, finish, inside_out_warnings, take_measurements};
use crate::util::{self, read_latin1, round1, s, write_latin1};

/// One .f3d (a whole file or a member of a .f3z).
pub struct Document {
    pub key: String,
    data: Vec<u8>,
    pub asm: Assembly,
    pub name: String,
    root_name: Option<String>,
    /// Component id -> the key of the document it is.
    xrefs: HashMap<u64, String>,
}

impl Document {
    fn new(key: &str, data: Vec<u8>) -> Result<Document> {
        let f = open_f3d(&data)?;
        let asm = read_assembly(&f)?;
        let designs: Vec<Value> =
            f.redirections.as_ref().and_then(|r| r.get("designs")).and_then(Value::as_array).cloned().unwrap_or_default();
        let mine: Vec<&Value> = designs.iter().filter(|d| d.get("targetFileName").and_then(Value::as_str) == Some(key)).collect();
        let mine = if mine.is_empty() { designs.iter().take(1).collect() } else { mine };
        let name = mine
            .first()
            .and_then(|d| d.get("displayName"))
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| util::stem(Path::new(key)));
        let root_name = asm.root.and_then(|r| asm.components.get(&r)).map(|c| c.name.clone());
        Ok(Document { key: key.to_owned(), data, asm, name, root_name, xrefs: HashMap::new() })
    }

    fn children(&self, cid: u64) -> impl Iterator<Item = &super::f3d::Placement> {
        self.asm.placements.iter().filter(move |p| p.parent == Some(cid))
    }
}

type Docs = IndexMap<String, Document>;

/// A component's name without the document version Fusion appends to it ("LTC 10Kg Tool v1"):
/// the component that stands for another document of a .f3z carries the version it links to,
/// while that document's own root carries its saved one ("LTC 10Kg Tool v0").
fn unversioned(name: &str) -> &str {
    let trimmed = name.trim_end();
    if let Some((base, v)) = trimmed.rsplit_once(" v")
        && !v.is_empty()
        && v.bytes().all(|b| b.is_ascii_digit())
    {
        return base;
    }
    trimmed
}

fn load(src: &Path) -> Result<(String, Docs)> {
    let mut docs: Docs = IndexMap::new();
    let root_key = if util::suffix_lower(src) == ".f3z" {
        let mut z = zip::ZipArchive::new(std::fs::File::open(src)?)?;
        let mut manifest = String::new();
        z.by_name("Manifest.json")
            .map_err(|_| anyhow!("There is no item named 'Manifest.json' in the archive"))?
            .read_to_string(&mut manifest)?;
        let root_name = serde_json::from_str::<Value>(&manifest)?.get("root").and_then(Value::as_str).map(str::to_owned);
        for i in 0..z.len() {
            let mut f = z.by_index(i)?;
            let name = f.name().map(|n| n.into_owned()).unwrap_or_default();
            if name.to_lowercase().ends_with(".f3d") && !name.contains('/') {
                let mut data = Vec::new();
                f.read_to_end(&mut data)?;
                drop(f);
                docs.insert(name.clone(), Document::new(&name, data)?);
            }
        }
        match root_name.filter(|r| docs.contains_key(r)) {
            Some(r) => r,
            None => return Err(F3dError("the archive has no root design".into()).into()),
        }
    } else {
        let key = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        docs.insert(key.clone(), Document::new(&key, std::fs::read(src)?)?);
        key
    };
    let mut by_root: HashMap<String, String> = HashMap::new();
    for d in docs.values() {
        if let Some(r) = d.root_name.as_deref().map(unversioned).filter(|r| !r.is_empty()) {
            by_root.insert(r.to_owned(), d.key.clone());
        }
    }
    let keys: Vec<String> = docs.keys().cloned().collect();
    for k in keys {
        let d = &docs[&k];
        let mut xrefs = HashMap::new();
        for (cid, c) in &d.asm.components {
            if let Some(other) = by_root.get(unversioned(&c.name))
                && Some(*cid) != d.asm.root
                && *other != d.key
            {
                xrefs.insert(*cid, other.clone());
            }
        }
        docs[&k].xrefs = xrefs;
    }
    Ok((root_key, docs))
}

/// Every placed component, document by document: visit(doc, cid, world matrix, path).
fn walk(docs: &Docs, root: &str, visit: &mut dyn FnMut(&Document, u64, &Mat4, &str) -> Result<()>) -> Result<()> {
    #[allow(clippy::too_many_arguments)]
    fn go(
        docs: &Docs,
        d: &Document,
        cid: Option<u64>,
        world: Mat4,
        path: &str,
        depth: usize,
        frames: usize,
        visit: &mut dyn FnMut(&Document, u64, &Mat4, &str) -> Result<()>,
    ) -> Result<()> {
        if frames > 900 {
            return Err(anyhow!("maximum recursion depth exceeded"));
        }
        let cid = cid.ok_or_else(|| anyhow!("KeyError: None"))?;
        visit(d, cid, &world, path)?;
        if depth > 16 {
            return Ok(());
        }
        for p in d.children(cid) {
            let child = matmul4(&world, &p.matrix);
            let target = p.target.and_then(|t| d.asm.components.get(&t).map(|c| (t, c)));
            let name = target.map_or("?", |(_, c)| c.name.as_str());
            let sub = format!("{path}/{name}");
            if let Some(other) = p.target.and_then(|t| d.xrefs.get(&t)).and_then(|k| docs.get(k)) {
                go(docs, other, other.asm.root, child, &sub, depth + 1, frames + 1, visit)?;
            } else if let Some((t, _)) = target {
                go(docs, d, Some(t), child, &sub, depth, frames + 1, visit)?;
            }
        }
        Ok(())
    }
    let d = &docs[root];
    if d.asm.root.is_some() {
        go(docs, d, d.asm.root, IDENTITY4, &d.name, 0, 0, visit)?;
    }
    Ok(())
}

/// The parts to make: every placed component with bodies of its own, and its file name.
fn plan(docs: &Docs, root: &str) -> Result<Vec<(String, u64, String)>> {
    let mut seen: IndexMap<(String, u64), ()> = IndexMap::new();
    walk(docs, root, &mut |d, cid, _w, _p| {
        let c = d.asm.components.get(&cid).ok_or_else(|| anyhow!("KeyError: {cid}"))?;
        if !c.bodies.is_empty() {
            seen.entry((d.key.clone(), cid)).or_insert(());
        }
        Ok(())
    })?;
    let mut by_doc: IndexMap<String, Vec<u64>> = IndexMap::new();
    for (key, cid) in seen.keys() {
        by_doc.entry(key.clone()).or_default().push(*cid);
    }
    let mut used = HashSet::new();
    let mut out = Vec::new();
    for (key, mut cids) in by_doc {
        cids.sort(); // stable, as Python's sorted
        for cid in cids {
            let stem = stem_for(&docs[&key].asm.components[&cid].name, &mut used);
            out.push((key.clone(), cid, stem));
        }
    }
    Ok(out)
}

/// The parts a conversion will make, in the order it makes them (for listing them first).
pub fn part_names(src: &Path) -> Result<Vec<String>> {
    let (root, docs) = load(src)?;
    Ok(plan(&docs, &root)?.into_iter().map(|(_, _, stem)| stem).collect())
}

fn geometry(doc: &Document, cadmpeg: &Path, build: &Path, hooks: &Hooks) -> Result<Option<StepFile>> {
    let source = build.join("documents").join(&doc.key);
    if let Some(dir) = source.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&source, &doc.data)?;
    let step = build.join(format!("{}.all.step", util::stem(Path::new(&doc.key))));
    hooks.step(&format!("leyendo la geometría de {}", doc.name));
    let out = util::run(cadmpeg, &["convert", &s(&source), "-o", &s(&step), "--force", "--allow-errors"], Duration::from_secs(3600))?;
    if !step.is_file() {
        let all = format!("{}{}", out.stdout, out.stderr);
        let last = util::last_line(&all).unwrap_or_else(|| out.status.unwrap_or(-1).to_string());
        hooks.log(&format!("  cadmpeg no ha podido leer {}: {last}", doc.name));
        return Ok(None);
    }
    Ok(Some(StepFile::new(&read_latin1(&step)?)))
}

/// Component id -> its bodies in the STEP file, and each body's name (by solid id).
type Assigned = (HashMap<u64, Vec<i64>>, HashMap<i64, String>);

fn assign(doc: &Document, step: &StepFile, wanted: &HashSet<u64>) -> Result<Assigned> {
    let mut order: Vec<(u64, &super::f3d::Body)> = Vec::new();
    for (cid, c) in &doc.asm.components {
        for b in &c.bodies {
            order.push((*cid, b));
        }
    }
    order.sort_by(|a, b| (a.1.brep.as_str(), a.1.selector).cmp(&(b.1.brep.as_str(), b.1.selector))); // stable
    let items = step.solids()?; // by entity id: the order cadmpeg wrote them
    let mut out: HashMap<u64, Vec<i64>> = HashMap::new();
    let mut names: HashMap<i64, String> = HashMap::new();
    if order.len() == items.len() {
        for ((cid, b), s) in order.iter().zip(&items) {
            if wanted.contains(cid) {
                out.entry(*cid).or_default().push(s.id);
                if let Some(n) = &b.name {
                    names.insert(s.id, n.clone());
                }
            }
        }
        return Ok((out, names));
    }
    // cadmpeg left some body out: each solid to the cached box that holds it best
    let slots: Vec<(u64, super::f3d::Box3, Option<&String>)> =
        order.iter().filter_map(|(cid, b)| b.bbox.filter(|_| wanted.contains(cid)).map(|bx| (*cid, bx, b.name.as_ref()))).collect();
    let pmax = |a: f64, b: f64| if b > a { b } else { a };
    let mut pairs: Vec<(f64, i64, usize)> = Vec::new();
    for s in &items {
        let Some(sb) = s.bbox else { continue };
        for (i, (_, bx, _)) in slots.iter().enumerate() {
            let outside = util::py_sum((0..3).map(|k| pmax(0.0, bx.0[k] - sb.0[k]) + pmax(0.0, sb.1[k] - bx.1[k])));
            if outside <= 0.05 * 6.0 {
                let slack = util::py_sum((0..3).map(|k| (bx.1[k] - bx.0[k]) - (sb.1[k] - sb.0[k])));
                pairs.push((outside * 1000.0 + slack, s.id, i));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let (mut taken_s, mut taken_b) = (HashSet::new(), HashSet::new());
    for (_, sid, i) in pairs {
        if !taken_s.contains(&sid) && !taken_b.contains(&i) {
            taken_s.insert(sid);
            taken_b.insert(i);
            out.entry(slots[i].0).or_default().push(sid);
            if let Some(n) = slots[i].2 {
                names.insert(sid, n.clone());
            }
        }
    }
    Ok((out, names))
}

/// A STEP string's contents (ISO 10303-21): quotes and backslashes doubled, anything outside
/// printable ASCII as \X2\hhhh\X0\.
fn step_string(s: &str) -> String {
    let mut out = String::new();
    for u in s.encode_utf16() {
        match u {
            0x27 => out.push_str("''"),
            0x5c => out.push_str("\\\\"),
            0x20..=0x7e => out.push(u as u8 as char),
            _ => out.push_str(&format!("\\X2\\{u:04X}\\X0\\")),
        }
    }
    out
}

static SOLID_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^#(\d+)(\s*=\s*(?:MANIFOLD_SOLID_BREP|BREP_WITH_VOIDS)\s*\(\s*)'(?:[^']|'')*'").expect("solid pattern")
});

/// Give the solids the names Fusion shows for their bodies (SolidCraft names bodies after them).
fn name_solids(text: &str, names: &HashMap<i64, String>) -> String {
    SOLID_LINE
        .replace_all(text, |c: &regex::Captures| match c[1].parse::<i64>().ok().and_then(|id| names.get(&id)) {
            Some(n) => format!("#{}{}'{}'", &c[1], &c[2], step_string(n)),
            None => c[0].to_owned(),
        })
        .into_owned()
}

/// Fusion caches a box that holds the body but may be a little loose round curved faces: the
/// part must lie inside it and fill it to within half a percent of its size.
fn box_error(cached: ([f64; 3], [f64; 3]), got: ([f64; 3], [f64; 3])) -> Option<String> {
    let pmax = |a: f64, b: f64| if b > a { b } else { a };
    let worst = |f: &dyn Fn(usize) -> f64| (1..3).fold(f(0), |m, k| pmax(m, f(k)));
    let out = worst(&|k| pmax(cached.0[k] - got.0[k], got.1[k] - cached.1[k]));
    if out > BOX_TOLERANCE {
        return Some(format!("se sale {out:.2} mm de la caja que guarda Fusion"));
    }
    let size = worst(&|k| cached.1[k] - cached.0[k]);
    let short = worst(&|k| pmax(got.0[k] - cached.0[k], cached.1[k] - got.1[k]));
    if short > pmax(BOX_TOLERANCE, 0.005 * size) {
        return Some(format!("le faltan {short:.2} mm para llenar la caja que guarda Fusion"));
    }
    None
}

/// The solid by its entity id, or by its name once it has one.
static OPEN_BODY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^solid (?:#(\d+)|'(.*)'): imported as an open body").expect("warning pattern"));

/// What SolidCraft could not read well. Not an open body that is so on purpose (a surface) or
/// only to SolidCraft (closed in the file: parts that touch at a corner).
fn import_warnings(results: &[Value], step: &StepFile, solids: &[i64], names: &HashMap<i64, String>) -> Vec<String> {
    let named = |name: &str| solids.iter().copied().find(|id| names.get(id).is_some_and(|n| n == name));
    let not_open = |w: &str| {
        OPEN_BODY
            .captures(w)
            .and_then(|m| m.get(1).and_then(|id| id.as_str().parse::<i64>().ok()).or_else(|| m.get(2).and_then(|n| named(n.as_str()))))
            .is_some_and(|id| step.kind.get(&id).map(String::as_str) == Some("SHELL_BASED_SURFACE_MODEL") || step.closed(id))
    };
    for r in results {
        if let Some(obj) = r.as_object().filter(|o| o.contains_key("created_bodies")) {
            let warnings = obj.get("warnings").and_then(Value::as_array).cloned().unwrap_or_default();
            return warnings
                .iter()
                .filter_map(|w| w.as_str())
                .filter(|w| !w.contains("could not be meshed") && !not_open(w))
                .map(str::to_owned)
                .collect();
        }
    }
    Vec::new()
}

fn bounds(boxes: &[([f64; 3], [f64; 3])]) -> ([f64; 3], [f64; 3]) {
    let mut lo = boxes[0].0;
    let mut hi = boxes[0].1;
    for b in &boxes[1..] {
        for k in 0..3 {
            if b.0[k] < lo[k] {
                lo[k] = b.0[k];
            }
            if b.1[k] > hi[k] {
                hi[k] = b.1[k];
            }
        }
    }
    (lo, hi)
}

#[allow(clippy::too_many_arguments)]
fn part(
    doc: &Document,
    cid: u64,
    solids: &[i64],
    names: &HashMap<i64, String>,
    step: &StepFile,
    stem: &str,
    out: &Path,
    cli: &Path,
    src: &Path,
    hooks: &Hooks,
) -> PartResult {
    let t0 = Instant::now();
    let comp = &doc.asm.components[&cid];
    let mut res = PartResult::new(stem, Some(s(src)));
    let build = out.join("build");
    let design = out.join("parts").join(format!("{stem}.solidcraft"));
    let _ = std::fs::create_dir_all(out.join("parts"));
    let attempt = (|| -> Result<()> {
        let part_step = build.join(format!("{stem}.step"));
        hooks.step("separando sus sólidos");
        let keep: HashSet<i64> = solids.iter().copied().collect();
        let (text, done) = clean(&step.keep_only(&keep)?, 1e-3)?;
        write_latin1(&part_step, &name_solids(&text, names))?;
        res.step = Some(s(&part_step));
        res.repair = Some(format!("lectura directa{}", if done.seams > 0 { format!(" ({} costuras)", done.seams) } else { String::new() }));
        let mut script = Script::default();
        script.add("file.new", json!({ "name": stem }));
        script.add("file.insert_step", json!({ "path": s(&part_step) }));
        script.add(
            "document.metadata.set",
            json!({"key": METADATA_KEY, "value": {"source": s(src), "source_cad": "Fusion", "document": doc.name, "component": comp.name}}),
        );
        script.add("file.save_as", json!({ "path": s(&design) }));
        // its bodies as SolidCraft holds them: what the assembly brings in
        let design_step = build.join(format!("{stem}.design.step"));
        script.add("file.export", json!({ "path": s(&design_step) }));
        hooks.step("importándola en SolidCraft");
        let results = sc::run(cli, &script.write(&build.join(format!("{stem}.part.json")))?)?;
        let problems = import_warnings(&results, step, solids, names);
        if let Some(first) = problems.first() {
            let more = if problems.len() > 1 { format!(" (y {} más)", problems.len() - 1) } else { String::new() };
            res.warnings.push(format!("SolidCraft avisa al leerla: {first}{more}"));
        }
        hooks.step("midiéndola");
        let measured = sc::evaluate(cli, &design)?;
        res.design = Some(s(&design));
        take_measurements(&mut res, &measured);
        let boxes = comp.boxes();
        let got = Measured(&measured).bbox()?;
        // with a body's box missing their union is too small to compare with
        if let Some(got) = got.filter(|_| !boxes.is_empty() && boxes.len() == comp.bodies.len()) {
            let err = box_error(bounds(&boxes), got);
            res.matches_source = Some(err.is_none() && problems.is_empty());
            if let Some(err) = err {
                res.warnings.push(format!("no coincide con Fusion: {err}"));
            }
        } else if !problems.is_empty() {
            res.matches_source = Some(false);
        }
        if solids.len() < comp.bodies.len() {
            res.warnings.push(format!("{} de {} cuerpos no se han podido leer", comp.bodies.len() - solids.len(), comp.bodies.len()));
        }
        inside_out_warnings(&mut res, &measured);
        if !res.ok {
            res.warnings.push("la pieza convertida no tiene sólidos".into());
        }
        let sketches = doc.asm.sketches.get(&cid).map(Vec::as_slice).unwrap_or(&[]);
        let timeline = doc.asm.timelines.get(&cid).map(Vec::as_slice).unwrap_or(&[]);
        let rebuilt = if timeline.iter().any(|f| matches!(f, Feature::Box { .. } | Feature::Extrude { .. })) {
            let meta = json!({"source": s(src), "source_cad": "Fusion", "document": doc.name, "component": comp.name});
            match from_timeline(timeline, sketches, &read_latin1(&part_step)?, &measured, stem, &build, cli, meta, hooks) {
                Ok(t) => {
                    std::fs::copy(&t.design, &design)?;
                    std::fs::copy(&t.step, &design_step)?;
                    res.repair = Some(format!("línea temporal ({} operaciones)", t.features));
                    true
                }
                Err(why) => {
                    res.warnings.push(format!("línea temporal no reproducida: {why}"));
                    false
                }
            }
        } else {
            false
        };
        if !rebuilt && !sketches.is_empty() {
            hooks.step("añadiendo sus bocetos");
            add_sketches(&mut res, sketches, &design, &build, stem, cli);
        }
        Ok(())
    })();
    if let Err(e) = attempt {
        res.warnings.push(e.to_string());
    }
    res.seconds = round1(t0.elapsed().as_secs_f64());
    hooks.log(&res.log_line());
    res
}

/// A body as `solidcraft-cli eval` measures it.
struct Measure {
    name: String,
    bbox: ([f64; 3], [f64; 3]),
    volume: f64,
}

fn measures(measured: &Value) -> Vec<Measure> {
    Measured(measured)
        .bodies()
        .iter()
        .filter_map(|b| {
            let corner = |k: &str| -> Option<[f64; 3]> {
                let a = b.get("bbox")?.get(k)?.as_array()?;
                Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
            };
            Some(Measure {
                name: b.get("name")?.as_str()?.to_owned(),
                bbox: (corner("min")?, corner("max")?),
                volume: b.get("volume_mm3")?.as_f64()?,
            })
        })
        .collect()
}

fn holds(outer: &([f64; 3], [f64; 3]), inner: &([f64; 3], [f64; 3]), tol: f64) -> bool {
    (0..3).all(|k| inner.0[k] >= outer.0[k] - tol && inner.1[k] <= outer.1[k] + tol)
}

fn box_volume(b: &([f64; 3], [f64; 3])) -> f64 {
    (0..3).map(|k| (b.1[k] - b.0[k]).max(0.0)).product()
}

/// A part made again from its timeline: the design, its bodies as STEP, and how many features.
struct Timeline {
    design: PathBuf,
    step: PathBuf,
    features: usize,
}

/// Build a part from its timeline and give it the imported part's bodies: SolidCraft makes a body
/// per extruded profile where Fusion joins profiles that touch, so the bodies are joined by the
/// imported body that holds them and renamed after it. Err says why the part is not the same.
#[allow(clippy::too_many_arguments)]
fn from_timeline(
    timeline: &[Feature],
    sketches: &[Sketch],
    step_text: &str,
    reference: &Value,
    stem: &str,
    build: &Path,
    cli: &Path,
    meta: Value,
    hooks: &Hooks,
) -> std::result::Result<Timeline, String> {
    const TOL: f64 = 0.02; // mm
    hooks.step("leyendo su línea temporal");
    let rebuilt = rebuild(timeline, sketches, &Step::new(step_text))?;
    hooks.step(&format!("rehaciendo su línea temporal en SolidCraft ({} operaciones)", rebuilt.features));
    let path = build.join(format!("{stem}.timeline.solidcraft"));
    let mut script = Script::default();
    script.add("file.new", json!({ "name": stem }));
    script.commands.extend(rebuilt.script.commands);
    script.add("document.metadata.set", json!({"key": METADATA_KEY, "value": meta}));
    // measured in the same run: each run or eval computes the whole part again
    script.add("document.inspect", json!({ "measure": true }));
    script.add("file.save_as", json!({ "path": s(&path) }));
    let fail = |e: anyhow::Error| format!("SolidCraft: {e}");
    let results = sc::run(cli, &script.write(&build.join(format!("{stem}.timeline.json"))).map_err(fail)?).map_err(fail)?;
    let got = measures(inspection(&results).ok_or("SolidCraft no ha medido la pieza")?);
    let want = measures(reference);
    // each rebuilt body to the smallest imported body that holds it
    let mut groups: Vec<Vec<&Measure>> = vec![Vec::new(); want.len()];
    for b in &got {
        let owner = (0..want.len())
            .filter(|&i| holds(&want[i].bbox, &b.bbox, TOL))
            .min_by(|&i, &j| box_volume(&want[i].bbox).total_cmp(&box_volume(&want[j].bbox)))
            .ok_or_else(|| format!("el cuerpo {} no está en ninguno de la pieza", b.name))?;
        groups[owner].push(b);
    }
    for (w, g) in want.iter().zip(&groups) {
        let volume: f64 = g.iter().map(|b| b.volume).sum();
        let mut union = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for b in g {
            for k in 0..3 {
                union.0[k] = union.0[k].min(b.bbox.0[k]);
                union.1[k] = union.1[k].max(b.bbox.1[k]);
            }
        }
        let same_box = (0..3).all(|k| (union.0[k] - w.bbox.0[k]).abs() <= TOL && (union.1[k] - w.bbox.1[k]).abs() <= TOL);
        if g.is_empty() || !same_box || (volume - w.volume).abs() > 0.01 * w.volume.abs().max(0.1) {
            return Err(format!("{} no sale igual ({:.3} mm³ frente a {:.3})", w.name, volume, w.volume));
        }
    }
    // name each group as Fusion does and join it. The names go into the design itself, in the
    // features that make the bodies: renaming through SolidCraft recomputes the whole part for
    // each body. The bodies a join takes in get names of their own, so none clashes with a
    // name from Fusion.
    let mut names: HashMap<&str, String> = HashMap::new();
    let mut joins: Vec<(String, Vec<String>)> = Vec::new();
    for (i, (g, w)) in groups.iter().zip(&want).enumerate() {
        names.insert(&g[0].name, w.name.clone());
        let tools: Vec<String> = (1..g.len()).map(|j| format!("__craftify{i}_{j}")).collect();
        for (b, t) in g[1..].iter().zip(&tools) {
            names.insert(&b.name, t.clone());
        }
        if !tools.is_empty() {
            joins.push((w.name.clone(), tools));
        }
    }
    hooks.step(&format!("nombrando sus {} cuerpos y uniendo los que Fusion une", want.len()));
    name_bodies(&path, &results, &names, &joins).map_err(|e| format!("no se pueden nombrar los cuerpos: {e}"))?;
    // one more computation of the part, which also gives what the assembly needs (its bodies as
    // STEP) and saves the design with its cache, so that opening it is quick
    hooks.step("comprobando los cuerpos y guardando la pieza");
    let step = build.join(format!("{stem}.timeline.step"));
    let mut script = Script::default();
    script.add("doc.open", json!({ "path": s(&path) }));
    script.add("document.inspect", json!({ "measure": true }));
    script.add("file.export", json!({ "path": s(&step) }));
    script.add("file.save_as", json!({ "path": s(&path) }));
    let results = sc::run(cli, &script.write(&build.join(format!("{stem}.timeline.check.json"))).map_err(fail)?).map_err(fail)?;
    let after = measures(inspection(&results).ok_or("SolidCraft no ha medido la pieza")?);
    let mut a: Vec<&str> = after.iter().map(|b| b.name.as_str()).collect();
    let mut b: Vec<&str> = want.iter().map(|b| b.name.as_str()).collect();
    a.sort_unstable();
    b.sort_unstable();
    let (va, vb): (f64, f64) = (after.iter().map(|x| x.volume).sum(), want.iter().map(|x| x.volume).sum());
    if a != b || (va - vb).abs() > 0.001 * vb.abs().max(1.0) {
        return Err(format!("los cuerpos no quedan iguales ({} frente a {})", after.len(), want.len()));
    }
    Ok(Timeline { design: path, step, features: rebuilt.features })
}

/// What `document.inspect` measured in a run's results.
fn inspection(results: &[Value]) -> Option<&Value> {
    results.iter().rev().find(|r| r.get("bodies").is_some() && r.get("revision").is_some())
}

/// Write body names and joins into a saved design: each feature lists the names of the bodies it
/// makes (`body_names`, in the order of the run's `created_bodies`), and a join is a `combine`
/// feature at the end.
fn name_bodies(design: &Path, results: &[Value], names: &HashMap<&str, String>, joins: &[(String, Vec<String>)]) -> Result<()> {
    let created: HashMap<u64, Vec<String>> = results
        .iter()
        .filter_map(|r| {
            let made: Vec<String> = r.get("created_bodies")?.as_array()?.iter().filter_map(|b| Some(b.as_str()?.to_owned())).collect();
            (!made.is_empty()).then_some((r.get("feature")?.as_u64()?, made))
        })
        .collect();
    edit_document(design, |doc| {
        let features = doc.get_mut("features").and_then(Value::as_array_mut).ok_or_else(|| anyhow!("a design without features"))?;
        let mut named = 0;
        for f in features.iter_mut() {
            let Some(made) = f.get("id").and_then(Value::as_u64).and_then(|id| created.get(&id)) else { continue };
            let new: Vec<String> = made.iter().map(|b| names.get(b.as_str()).cloned().unwrap_or_else(|| b.clone())).collect();
            named += made.iter().filter(|b| names.contains_key(b.as_str())).count();
            f["body_names"] = json!(new);
        }
        if named != names.len() {
            return Err(anyhow!("{} de {} cuerpos sin la operación que los hace", names.len() - named, names.len()));
        }
        let mut next = doc["next_id"].as_u64().ok_or_else(|| anyhow!("a design without next_id"))?;
        let features = doc["features"].as_array_mut().expect("checked above");
        for (k, (target, tools)) in joins.iter().enumerate() {
            features.push(json!({
                "id": next, "name": format!("Combine{}", k + 1), "type": "combine", "operation": "join",
                "target": target, "tools": tools, "keep_tools": false, "suppressed": false,
            }));
            next += 1;
        }
        doc["next_id"] = json!(next);
        Ok(())
    })
}

/// The component's sketches into its design, in a run of their own: SolidCraft writes nothing
/// when a command fails, so a sketch it refuses leaves the part as it was.
fn add_sketches(res: &mut PartResult, sketches: &[Sketch], design: &Path, build: &Path, stem: &str, cli: &Path) {
    let mut script = Script::default();
    script.add("doc.open", json!({ "path": s(design) }));
    for sk in sketches {
        sketch::add_to(&mut script, sk);
    }
    script.add("file.save_as", json!({ "path": s(design) }));
    let run = script.write(&build.join(format!("{stem}.sketches.json"))).and_then(|p| sc::run(cli, &p));
    if let Err(e) = run {
        res.warnings.push(format!("no se han podido pasar los bocetos: {e}"));
        return;
    }
    let skipped: usize = sketches.iter().map(|s| s.skipped).sum();
    if skipped > 0 {
        res.warnings.push(format!("{skipped} curvas de los bocetos aún no se pueden pasar (elipses, textos, aristas proyectadas…)"));
    }
}

pub fn convert_fusion_offline(src: &Path, out_root: &Path, cli: &Path, cadmpeg: &Path, picture: bool, hooks: &Hooks) -> Result<Report> {
    let t0 = Instant::now();
    let name = util::stem(src);
    let out = out_root.join(&name);
    let build = out.join("build");
    std::fs::create_dir_all(&build)?;
    let mut report = Report::new(s(src));
    hooks.log(&format!("{name}: diseño de Fusion"));
    let (root, docs) = load(src)?;
    hooks.log(&format!("  {} documento(s): {}", docs.len(), docs.values().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ")));

    let plan = plan(&docs, &root)?;
    hooks.tell(Stage::Start, 0, plan.len(), None);
    let mut designs: HashMap<(String, u64), PathBuf> = HashMap::new();
    let stems: HashMap<(String, u64), String> = plan.iter().map(|(k, c, s)| ((k.clone(), *c), s.clone())).collect();
    let mut steps: HashMap<String, Option<StepFile>> = HashMap::new();
    let mut assigned: HashMap<String, Assigned> = HashMap::new();
    for (n, (key, cid, stem)) in plan.iter().enumerate() {
        let d = &docs[key];
        if !steps.contains_key(key) {
            // each document's geometry once, when its first part comes
            let step = geometry(d, cadmpeg, &build, hooks)?;
            let wanted_here: HashSet<u64> = plan.iter().filter(|(k, _, _)| k == key).map(|(_, c, _)| *c).collect();
            let a = match &step {
                Some(st) => assign(d, st, &wanted_here)?,
                None => Default::default(),
            };
            assigned.insert(key.clone(), a);
            steps.insert(key.clone(), step);
        }
        let (by_comp, names) = &assigned[key];
        let solids = by_comp.get(cid).cloned().unwrap_or_default();
        let r = match steps[key].as_ref() {
            Some(step) if !solids.is_empty() => {
                let r = part(d, *cid, &solids, names, step, stem, &out, cli, src, hooks);
                if r.ok
                    && let Some(des) = &r.design
                {
                    designs.insert((key.clone(), *cid), PathBuf::from(des));
                }
                r
            }
            _ => {
                hooks.log(&format!("  ERR {stem}: no se ha podido leer su geometría"));
                PartResult::with_warning(stem, Some(s(src)), "no se ha podido leer su geometría")
            }
        };
        hooks.tell(Stage::Part, n + 1, plan.len(), Some(&r));
        report.parts.push(r);
    }
    let total = plan.len();
    let needed: HashSet<(String, u64)> = plan.iter().map(|(k, c, _)| (k.clone(), *c)).collect();

    let mut placements: Vec<Placement> = Vec::new();
    let mut wanted = 0;
    walk(&docs, &root, &mut |d, cid, w, path| {
        let key = (d.key.clone(), cid);
        if needed.contains(&key) {
            wanted += 1;
        }
        if let Some(design) = designs.get(&key) {
            let m = [w[0], w[1], w[2], w[4], w[5], w[6], w[8], w[9], w[10]];
            let step = build.join(format!("{}.design.step", stems[&key]));
            placements.push(Placement {
                stem: stems[&key].clone(),
                design: design.clone(),
                step: step.is_file().then_some(step),
                rotation: m,
                translation_mm: [w[3], w[7], w[11]],
                label: path.to_owned(),
            });
        }
        Ok(())
    })?;
    report.instances = wanted;
    hooks.tell(Stage::Assembly, total, total, None);
    let lone = (placements.len() == 1).then(|| &placements[0]);
    if wanted == 1 && lone.is_some_and(|l| l.rotation == IDENTITY && l.translation_mm.iter().all(|x| *x == 0.0)) {
        report.output = Some(s(&placements[0].design)); // a single part where the design has it
        report.placed = 1;
    } else if !placements.is_empty() {
        let design = out.join(format!("{name}.solidcraft"));
        let mut meta = Map::new();
        meta.insert("source".into(), json!(s(src)));
        meta.insert("source_cad".into(), json!("Fusion"));
        // a linked part brings only its bodies: its sketches are drawn again in its component
        let by_stem: HashMap<&str, &(String, u64)> = stems.iter().map(|(k, v)| (v.as_str(), k)).collect();
        let sketches_of = |stem: &str| by_stem.get(stem).and_then(|(key, cid)| docs[key].asm.sketches.get(cid)).filter(|s| !s.is_empty());
        let (with, _) = assembly_script_with(&name, &placements, &design, meta.clone(), |stem, script| match sketches_of(stem) {
            Some(sks) => {
                sks.iter().for_each(|sk| sketch::add_to(script, sk));
                true
            }
            None => false,
        });
        hooks.step(&format!("colocando {} piezas", placements.len()));
        let run = |script: &Script| {
            script
                .write(&build.join(format!("{name}.assembly.json")))
                .and_then(|p| sc::run(cli, &p))
                .and_then(|r| link_parts(&design, &placements).map(|_| r))
        };
        let mut done = run(&with);
        if let Err(e) = &done
            && placements.iter().any(|p| sketches_of(&p.stem).is_some())
        {
            report.warnings.push(format!("el ensamblaje va sin bocetos: {e}"));
            done = run(&assembly_script(&name, &placements, &design, meta).0);
        }
        match done {
            Ok(_) => {
                report.output = Some(s(&design));
                report.placed = placements.len();
            }
            Err(e) => report.warnings.push(format!("ensamblaje: {e}")),
        }
    } else {
        report.warnings.push("el diseño no tiene sólidos que se hayan podido leer".into());
    }

    if let Some(png) = preview_png(src) {
        std::fs::write(out.join(PREVIEW_FUSION), png)?;
    }
    finish(&mut report, &out, cli, picture, hooks, t0, total)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solids_take_their_bodies_names() {
        let text = "#5 = MANIFOLD_SOLID_BREP('',#4);\n#9 = BREP_WITH_VOIDS('old',#8,(#7));\n#12 = MANIFOLD_SOLID_BREP('',#11);\n";
        let names = HashMap::from([(5, "Título d'A".to_owned()), (9, "Cuerpo2".to_owned())]);
        assert_eq!(
            name_solids(text, &names),
            "#5 = MANIFOLD_SOLID_BREP('T\\X2\\00ED\\X0\\tulo d''A',#4);\n#9 = BREP_WITH_VOIDS('Cuerpo2',#8,(#7));\n#12 = MANIFOLD_SOLID_BREP('',#11);\n"
        );
    }

    #[test]
    fn document_versions_do_not_count_in_names() {
        assert_eq!(unversioned("LTC 10Kg Tool v1"), "LTC 10Kg Tool");
        assert_eq!(unversioned("LTC 10Kg Tool v0"), "LTC 10Kg Tool");
        assert_eq!(unversioned("LTC 10Kg v12"), "LTC 10Kg");
        assert_eq!(unversioned("Tapa v2b"), "Tapa v2b");
        assert_eq!(unversioned("Servo v"), "Servo v");
        assert_eq!(unversioned("LINK4-CR5A"), "LINK4-CR5A");
    }
}

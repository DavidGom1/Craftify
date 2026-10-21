//! SolidCraft assembly designs from part designs and their placements (any source CAD).
//! Port of `solidcraft/assembly.py`.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::script::{METADATA_KEY, Script};
use super::transform::{Matrix, Vec3, axis_angle};
use crate::util::s;

pub struct Placement {
    /// The part design's name (component name in SolidCraft).
    pub stem: String,
    pub design: PathBuf,
    /// For column vectors, in SolidCraft's world.
    pub rotation: Matrix,
    pub translation_mm: Vec3,
    /// The source CAD's name for this instance.
    pub label: String,
}

/// Each occurrence's moves: turned about the world origin, then translated.
pub fn add_moves(s: &mut Script, occ: &str, rotation: &Matrix, translation: &Vec3) {
    let (axis, angle) = axis_angle(rotation);
    if angle > 1e-12 {
        s.add("occurrence.move", json!({"occurrence": occ, "space": "world", "axis": axis, "angle": angle, "origin": [0, 0, 0]}));
    }
    if translation.iter().any(|x| x.abs() > 1e-9) {
        s.add("occurrence.move", json!({"occurrence": occ, "space": "world", "translate": translation}));
    }
}

/// A script that links every part design once and places one occurrence per placement.
///
/// A copy takes the current placement of the occurrence it copies, so every occurrence is made
/// while the first one is still at the origin, and only then are they moved.
pub fn assembly_script(
    name: &str,
    placements: &[Placement],
    out_design: &Path,
    metadata: Map<String, Value>,
) -> (Script, IndexMap<String, String>) {
    assembly_script_with(name, placements, out_design, metadata, |_, _| false)
}

/// As [`assembly_script`], and `inside(stem, script)` adds what each part has besides its bodies
/// (a linked design brings only those): it runs with that part's component active, before any
/// occurrence moves, and says whether it added anything.
pub fn assembly_script_with(
    name: &str,
    placements: &[Placement],
    out_design: &Path,
    metadata: Map<String, Value>,
    mut inside: impl FnMut(&str, &mut Script) -> bool,
) -> (Script, IndexMap<String, String>) {
    let mut s = Script::default();
    s.add("file.new", json!({ "name": name }));
    let mut counts: IndexMap<&str, usize> = IndexMap::new();
    let mut todo = Vec::new();
    let mut mapping = IndexMap::new();
    for p in placements {
        let n = counts.get(p.stem.as_str()).copied().unwrap_or(0) + 1;
        counts.insert(&p.stem, n);
        if n == 1 {
            s.add("component.insert_design", json!({"path": s_(&p.design), "name": p.stem}));
        } else {
            s.add("occurrence.copy", json!({ "occurrence": format!("{}:1", p.stem) }));
        }
        let occ = format!("{}:{n}", p.stem);
        mapping.insert(if p.label.is_empty() { occ.clone() } else { p.label.clone() }, occ.clone());
        todo.push((p, occ));
    }
    let mut any = false;
    for &stem in counts.keys() {
        let mut part = Script::default();
        if inside(stem, &mut part) {
            s.add("component.activate", json!({ "component": stem }));
            s.commands.extend(part.commands);
            any = true;
        }
    }
    if any {
        s.add("component.activate", json!({ "component": "root" }));
    }
    for (p, occ) in &todo {
        add_moves(&mut s, occ, &p.rotation, &p.translation_mm);
    }
    let mut meta = metadata;
    meta.insert("instances".into(), json!(mapping));
    s.add("document.metadata.set", json!({"key": METADATA_KEY, "value": meta}));
    s.add("file.save_as", json!({ "path": s_(out_design) }));
    (s, mapping)
}

fn s_(p: &Path) -> String {
    s(p)
}

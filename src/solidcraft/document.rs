//! A design written straight as its file, without SolidCraft computing it: the features a script
//! of commands would leave in `document.json`, as SolidCraft writes them. SolidCraft computes the
//! design when it opens it.
//!
//! Only the commands Craftify uses to rebuild a Fusion timeline are known: `solid.box`,
//! `solid.fillet`, a sketch (`sketch.create`, its lines, arcs, circles and control-point splines,
//! `sketch.finish`) and `solid.extrude`; any other is an error.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use super::script::METADATA_KEY;

/// The format SolidCraft 0.19 writes.
const FORMAT: &str = "solidcraft/30";

/// The features of a script's commands, numbered from `first_id`, and the index of the feature
/// each command made (None for those inside a sketch).
pub fn features(commands: &[Value], first_id: u64) -> Result<(Vec<Value>, Vec<Option<usize>>)> {
    let mut out: Vec<Value> = Vec::new();
    let mut made_by: Vec<Option<usize>> = Vec::with_capacity(commands.len());
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut sketch_ids: HashMap<String, u64> = HashMap::new();
    let mut sketch: Option<SketchJson> = None;
    for c in commands {
        let name = c["command"].as_str().unwrap_or_default();
        let p = &c["params"];
        let id = first_id + out.len() as u64;
        let mut numbered = |kind: &'static str| {
            let n = counts.entry(kind).or_insert(0);
            *n += 1;
            format!("{kind}{n}")
        };
        let feature = match name {
            "sketch.create" => {
                sketch = Some(SketchJson::new(p)?);
                made_by.push(None);
                continue;
            }
            "sketch.line" | "sketch.arc.center_point" | "sketch.circle.center" | "sketch.spline.control_point" => {
                sketch.as_mut().ok_or_else(|| anyhow!("{name} outside a sketch"))?.add(name, p)?;
                made_by.push(None);
                continue;
            }
            "sketch.finish" => {
                let sk = sketch.take().ok_or_else(|| anyhow!("sketch.finish outside a sketch"))?;
                sketch_ids.insert(sk.name.clone(), id);
                sk.feature(id)
            }
            "solid.box" => json!({
                "id": id, "name": numbered("Box"), "type": "box", "operation": "new_body",
                "length": p["length"], "width": p["width"], "height": p["height"], "corner": p["corner"],
                "suppressed": false,
            }),
            "solid.fillet" => json!({
                "id": id, "name": numbered("Fillet"), "type": "fillet", "edges": p["edges"], "radius": p["radius"],
                "suppressed": false,
            }),
            "solid.extrude" => {
                let on = p["sketch"].as_str().and_then(|s| sketch_ids.get(s)).ok_or_else(|| anyhow!("an extrude of an unknown sketch"))?;
                let loops: Vec<&Value> = p["profiles"].as_array().map(|a| a.iter().map(|x| &x["curves"]).collect()).unwrap_or_default();
                let mut extent = json!({"convention": "total_distance", "direction": p["direction"], "distance": p["distance"]});
                if let Some(s) = p.get("start_offset") {
                    extent["start_offset"] = s.clone();
                }
                let operation = match p["operation"].as_str() {
                    Some("cut") => "cut",
                    Some("new") => "new_body",
                    other => bail!("an extrude with operation {other:?}"),
                };
                json!({
                    "id": id, "name": numbered("Extrude"), "type": "extrude", "operation": operation, "sketch": on,
                    "extent": extent, "profiles": {"type": "curves", "loops": loops}, "suppressed": false,
                })
            }
            other => bail!("{other} cannot be written into a design yet"),
        };
        made_by.push(Some(out.len()));
        out.push(feature);
    }
    if sketch.is_some() {
        bail!("a sketch that is not finished");
    }
    Ok((out, made_by))
}

/// A sketch as SolidCraft keeps it: points numbered in one list (the origin first), curves named
/// by kind and number (`l1`, `a1`, `c1`, `s1`) and pointing at their points by index.
struct SketchJson {
    name: String,
    plane: Value,
    points: Vec<Value>,
    curves: Vec<Value>,
    counters: HashMap<&'static str, usize>,
}

impl SketchJson {
    fn new(p: &Value) -> Result<SketchJson> {
        let name = p["name"].as_str().ok_or_else(|| anyhow!("a sketch without a name"))?.to_owned();
        let pl = &p["plane"];
        Ok(SketchJson {
            name,
            plane: json!({"type": "custom", "plane": {"origin": pl["origin"], "x_dir": pl["x_dir"], "y_dir": pl["y_dir"]}}),
            points: vec![json!({"fixed": true, "id": "origin", "pos": [0.0, 0.0]})],
            curves: Vec::new(),
            counters: HashMap::new(),
        })
    }

    fn next(&mut self, kind: &'static str) -> usize {
        let n = self.counters.entry(kind).or_insert(0);
        *n += 1;
        *n
    }

    fn point(&mut self, id: String, pos: &Value) -> usize {
        self.points.push(json!({"fixed": false, "id": id, "pos": pos}));
        self.points.len() - 1
    }

    fn add(&mut self, command: &str, p: &Value) -> Result<()> {
        let curve = match command {
            "sketch.line" => {
                let id = format!("l{}", self.next("l"));
                let a = self.point(format!("{id}.start"), &p["points"][0]);
                let b = self.point(format!("{id}.end"), &p["points"][1]);
                json!({"id": id, "type": "line", "a": a, "b": b, "construction": p["construction"].as_bool().unwrap_or(false)})
            }
            "sketch.arc.center_point" => {
                let id = format!("a{}", self.next("a"));
                let c = self.point(format!("{id}.center"), &p["center"]);
                let a = self.point(format!("{id}.start"), &p["start"]);
                let b = self.point(format!("{id}.end"), &p["end"]);
                json!({"id": id, "type": "arc", "a": a, "b": b, "c": c, "construction": false})
            }
            "sketch.circle.center" => {
                let id = format!("c{}", self.next("c"));
                let c = self.point(format!("{id}.center"), &p["center"]);
                json!({"id": id, "type": "circle", "c": c, "r": p["radius"], "construction": false})
            }
            _ => {
                let id = format!("s{}", self.next("s"));
                let pts: Vec<usize> = p["points"]
                    .as_array()
                    .ok_or_else(|| anyhow!("a spline without points"))?
                    .iter()
                    .map(|q| {
                        let n = self.next("p");
                        self.point(format!("p{n}"), q)
                    })
                    .collect();
                json!({"id": id, "type": "spline", "control": true, "degree": 3, "pts": pts, "construction": false})
            }
        };
        self.curves.push(curve);
        Ok(())
    }

    fn feature(self, id: u64) -> Value {
        let counters: Map<String, Value> = self.counters.iter().map(|(k, v)| ((*k).to_owned(), json!(v))).collect();
        json!({
            "id": id, "name": self.name, "type": "sketch", "plane": self.plane,
            "sketch": {"constraints": [], "counters": counters, "curves": self.curves, "points": self.points},
            "suppressed": false,
        })
    }
}

/// Write a part design: its features, Craftify's metadata, and no cache (SolidCraft computes the
/// design when it opens it).
pub fn write_part(path: &Path, name: &str, features: Vec<Value>, metadata: Value) -> Result<()> {
    let next_id = features.iter().filter_map(|f| f["id"].as_u64()).max().unwrap_or(0) + 1;
    let doc = json!({
        "format": FORMAT, "name": name, "units": "mm", "params": [], "next_id": next_id,
        "metadata": { METADATA_KEY: metadata }, "features": features,
    });
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = zip::ZipWriter::new(std::fs::File::create(path)?);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    out.start_file("document.json", options)?;
    out.write_all(&serde_json::to_vec(&doc)?)?;
    out.start_file("container.json", options)?;
    out.write_all(br#"{"format":"solidcraft-container/1"}"#)?;
    out.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What SolidCraft 0.19 saved for the same commands (out/sk.json in the conversation that
    /// wrote this): the same points, curves and counters.
    #[test]
    fn a_sketch_as_solidcraft_saves_it() {
        let cmd = |c: &str, p: Value| json!({"command": c, "params": p});
        let commands = vec![
            cmd("sketch.create", json!({"plane": {"origin": [0, 0, 0.8], "x_dir": [-1, 0, 0], "y_dir": [0, -1, 0]}, "name": "Boceto9"})),
            cmd("sketch.line", json!({"points": [[0, 0], [10, 0]], "construction": false})),
            cmd("sketch.line", json!({"points": [[0, 1], [10, 1]], "construction": true})),
            cmd("sketch.arc.center_point", json!({"center": [5, 5], "start": [8, 5], "end": [5, 8]})),
            cmd("sketch.circle.center", json!({"center": [20, 20], "radius": 3})),
            cmd("sketch.spline.control_point", json!({"points": [[0, 10], [3, 13], [6, 13], [9, 10]]})),
            cmd("sketch.finish", json!({})),
        ];
        let (f, made_by) = features(&commands, 1).unwrap();
        assert_eq!(made_by, vec![None, None, None, None, None, None, Some(0)]);
        let sk = &f[0]["sketch"];
        assert_eq!(sk["counters"], json!({"l": 2, "a": 1, "c": 1, "s": 1, "p": 4}));
        assert_eq!(sk["curves"][0], json!({"a": 1, "b": 2, "construction": false, "id": "l1", "type": "line"}));
        assert_eq!(sk["curves"][2], json!({"a": 6, "b": 7, "c": 5, "construction": false, "id": "a1", "type": "arc"}));
        assert_eq!(sk["curves"][3], json!({"c": 8, "construction": false, "id": "c1", "r": 3, "type": "circle"}));
        assert_eq!(sk["curves"][4]["pts"], json!([9, 10, 11, 12]));
        assert_eq!(sk["points"][9], json!({"fixed": false, "id": "p1", "pos": [0, 10]}));
        assert_eq!(sk["points"].as_array().unwrap().len(), 13);
    }
}

//! A SolidWorks part as cadmpeg describes it (`cadmpeg dump`: CADIR JSON). Port of `sw/ir.py`
//! for what the converter uses: SolidWorks' own display mesh of the part.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use crate::mesh::Mesh;
use crate::solidcraft::script::truthy;
use crate::util::{self, last_line, s};

/// The part's CADIR model, from the cache when there is one.
pub fn load(path: &Path, cadmpeg: &Path, cache: Option<&Path>) -> Result<Value> {
    let model: Value = match cache {
        Some(c) if c.is_file() => serde_json::from_str(&std::fs::read_to_string(c)?)?,
        _ => {
            let out = util::run(cadmpeg, &["dump", &s(path)], Duration::from_secs(900))?;
            if out.status != Some(0) {
                bail!(last_line(&out.stderr).unwrap_or_else(|| format!("cadmpeg dump exited with {}", out.status.unwrap_or(-1))));
            }
            let model = serde_json::from_str(&out.stdout)?;
            if let Some(c) = cache {
                if let Some(dir) = c.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(c, &out.stdout)?;
            }
            model
        }
    };
    let model = match model.get("model") {
        Some(m) => m.clone(),
        None => model,
    };
    check(&model)?;
    Ok(model)
}

/// The indexing Python does on load (feature and sketch ids), which fails on the same models.
fn check(model: &Value) -> Result<()> {
    let list = |key: &str| model.get(key).filter(|v| truthy(v)).and_then(Value::as_array).cloned().unwrap_or_default();
    for f in list("features") {
        f.get("id").ok_or_else(|| anyhow!("KeyError: 'id'"))?;
    }
    for sk in list("sketches") {
        sk.get("id").ok_or_else(|| anyhow!("KeyError: 'id'"))?;
    }
    Ok(())
}

fn xyz(v: &Value) -> Result<[f64; 3]> {
    let k = |name: &str| v.get(name).and_then(Value::as_f64).ok_or_else(|| anyhow!("KeyError: '{name}'"));
    Ok([k("x")?, k("y")?, k("z")?])
}

/// SolidWorks' own display mesh of the part (outward normals).
pub fn mesh(model: &Value) -> Result<Mesh> {
    let mut out = Mesh::default();
    let empty = Vec::new();
    let tess = model.get("tessellations").and_then(Value::as_array).unwrap_or(&empty);
    for t in tess {
        let Some(m) = t.get("mesh").filter(|m| truthy(m)) else { continue };
        if m.get("kind").and_then(Value::as_str) != Some("shaded_strips") {
            continue;
        }
        for strip in m.get("strips").and_then(Value::as_array).unwrap_or(&empty) {
            let strip = strip.as_array().ok_or_else(|| anyhow!("a strip is not a list"))?;
            let mut pts = Vec::with_capacity(strip.len());
            let mut nrm = Vec::with_capacity(strip.len());
            for v in strip {
                pts.push(xyz(v.get("position").ok_or_else(|| anyhow!("KeyError: 'position'"))?)?);
                nrm.push(match v.get("normal").filter(|n| truthy(n)) {
                    Some(n) => Some(xyz(n)?),
                    None => None,
                });
            }
            for k in 0..pts.len().saturating_sub(2) {
                let ns: Vec<[f64; 3]> = nrm[k..k + 3].iter().flatten().copied().collect();
                let outward = (!ns.is_empty()).then(|| {
                    let sum = |i: usize| util::py_sum(ns.iter().map(|n| n[i]));
                    [sum(0), sum(1), sum(2)]
                });
                out.add(pts[k], pts[k + 1], pts[k + 2], outward);
            }
        }
    }
    Ok(out)
}

/// The display mesh of a part, read through the cache the converter keeps in `build/ir`.
pub fn part_mesh(src: &Path, cadmpeg: &Path, build: &Path) -> Result<Mesh> {
    let cache = build.join("ir").join(format!("{}.json", util::stem(src)));
    mesh(&load(src, cadmpeg, Some(&cache))?)
}

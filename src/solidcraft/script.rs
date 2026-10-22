//! Build SolidCraft command scripts and run them with solidcraft-cli. Port of
//! `solidcraft/script.py`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

use crate::util::{self, last_line, s};

pub const METADATA_KEY: &str = "com.hidroconta.cad2solidcraft";

/// A failure reported by solidcraft-cli itself (Python's `SolidCraftError`). Other failures
/// (a timeout, an unreadable result) are plain errors, as in Python.
#[derive(Debug)]
pub struct SolidCraftError(pub String);

impl std::fmt::Display for SolidCraftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SolidCraftError {}

pub fn is_solidcraft_error(e: &anyhow::Error) -> bool {
    e.is::<SolidCraftError>()
}

#[derive(Default)]
pub struct Script {
    pub commands: Vec<Value>,
}

impl Script {
    pub fn add(&mut self, command: &str, params: Value) -> &mut Self {
        let mut c = Map::new();
        c.insert("command".into(), Value::String(command.into()));
        c.insert("params".into(), params);
        self.commands.push(Value::Object(c));
        self
    }

    pub fn write(&self, path: &Path) -> Result<PathBuf> {
        util::write_json(path, &json!({ "commands": self.commands }))?;
        Ok(path.to_path_buf())
    }
}

/// solidcraft-cli: the one shipped with this program first (its `solidcraft` folder), then the
/// fetched one (packaging/solidcraft/bin), then SolidCraft's own if it is installed.
pub fn find_cli(explicit: Option<&str>) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.extend(explicit.map(PathBuf::from));
    candidates.extend(std::env::var_os("SOLIDCRAFT_CLI").map(PathBuf::from));
    if let Some(dir) = util::exe_dir() {
        candidates.push(dir.join("solidcraft").join("solidcraft-cli.exe"));
    }
    candidates.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/packaging/solidcraft/bin/solidcraft-cli.exe")));
    candidates.extend(util::which("solidcraft-cli"));
    for var in ["ProgramFiles", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(var) {
            let base = PathBuf::from(base);
            candidates.push(base.join("SolidCraft").join("solidcraft-cli.exe"));
            candidates.push(base.join("Programs").join("SolidCraft").join("solidcraft-cli.exe"));
        }
    }
    candidates
        .into_iter()
        .find(|c| !c.as_os_str().is_empty() && c.is_file())
        .ok_or_else(|| SolidCraftError("solidcraft-cli not found: it comes in Craftify's solidcraft folder".into()).into())
}

fn failure(out: &util::Output, fallback: String) -> anyhow::Error {
    let text = if out.stderr.trim().is_empty() { &out.stdout } else { &out.stderr };
    SolidCraftError(last_line(text).unwrap_or(fallback)).into()
}

/// Run a script; return each command's result.
pub fn run(cli: &Path, script_path: &Path) -> Result<Vec<Value>> {
    let results = script_path.with_extension("results.json");
    if results.exists() {
        std::fs::remove_file(&results)?;
    }
    let out = util::run(cli, &["run", &s(script_path), "--results", "--quiet", "--json-out", &s(&results)], Duration::from_secs(1800))?;
    if out.status != Some(0) {
        return Err(failure(&out, format!("solidcraft-cli exited with {}", out.status.unwrap_or(-1))));
    }
    if !results.exists() {
        return Ok(Vec::new());
    }
    let data: Value = serde_json::from_str(&std::fs::read_to_string(&results)?)?;
    Ok(match data {
        Value::Array(a) => a,
        other => vec![other],
    })
}

/// Measurements of a design (bodies, volume, area, bounding boxes).
pub fn evaluate(cli: &Path, design: &Path) -> Result<Value> {
    let out_path = design.with_extension("eval.json");
    let out = util::run(cli, &["eval", &s(design), "--json-out", &s(&out_path)], Duration::from_secs(600))?;
    if out.status != Some(0) {
        return Err(failure(&out, format!("solidcraft-cli eval exited with {}", out.status.unwrap_or(-1))));
    }
    let text = std::fs::read_to_string(&out_path).map_err(|e| anyhow!("{}: {e}", out_path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

pub fn snapshot(cli: &Path, design: &Path, png: &Path, view: &str, width: u32, height: u32) -> Result<()> {
    let (w, h) = (width.to_string(), height.to_string());
    let out = util::run(
        cli,
        &["snapshot", "--in", &s(design), "--out", &s(png), "--view", view, "--width", &w, "--height", &h, "--no-grid"],
        Duration::from_secs(600),
    )?;
    if out.status != Some(0) {
        return Err(failure(&out, "solidcraft-cli snapshot failed".into()));
    }
    Ok(())
}

/// What `solidcraft-cli eval` says about a design, as the converter reads it.
pub struct Measured<'a>(pub &'a Value);

impl Measured<'_> {
    pub fn body_count(&self) -> Option<i64> {
        self.0.get("body_count").and_then(Value::as_i64)
    }
    pub fn total_volume(&self) -> Option<f64> {
        self.0.get("total").and_then(|t| t.get("volume_mm3")).and_then(Value::as_f64)
    }
    pub fn bodies(&self) -> &[Value] {
        self.0.get("bodies").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
    }
    /// The box around every body that has one: (min, max).
    pub fn bbox(&self) -> Result<Option<([f64; 3], [f64; 3])>> {
        let boxes: Vec<&Value> = self.bodies().iter().filter_map(|b| b.get("bbox").filter(|x| truthy(x))).collect();
        if boxes.is_empty() {
            return Ok(None);
        }
        let comp = |b: &Value, key: &str, k: usize| -> Result<f64> {
            b.get(key).and_then(|v| v.get(k)).and_then(Value::as_f64).ok_or_else(|| anyhow!("KeyError: '{key}'"))
        };
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for k in 0..3 {
            for b in &boxes {
                lo[k] = lo[k].min(comp(b, "min", k)?);
                hi[k] = hi[k].max(comp(b, "max", k)?);
            }
        }
        Ok(Some((lo, hi)))
    }
    /// The volume held by bodies that are inside out.
    pub fn negative_volume(&self) -> f64 {
        util::py_sum(self.bodies().iter().filter_map(|b| b.get("volume_mm3").and_then(Value::as_f64)).filter(|v| *v < 0.0).map(|v| -v))
    }
}

/// Python's truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

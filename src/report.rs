//! What a conversion did: one result per part and the whole report (`report.json`).
//! Field names match the Python converter's, so either one's report reads the same.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PartResult {
    pub stem: String,
    pub source: Option<String>,
    pub design: Option<String>,
    pub step: Option<String>,
    pub ok: bool,
    pub volume_mm3: Option<f64>,
    pub bodies: Option<i64>,
    /// Which orientation repair was kept.
    pub repair: Option<String>,
    /// The source CAD's own figure (SolidWorks' display mesh, Fusion's exact volume).
    pub source_volume_mm3: Option<f64>,
    pub matches_source: Option<bool>,
    pub seconds: f64,
    pub warnings: Vec<String>,
}

impl PartResult {
    pub fn new(stem: &str, source: Option<String>) -> Self {
        PartResult { stem: stem.to_owned(), source, ..Default::default() }
    }

    pub fn with_warning(stem: &str, source: Option<String>, warning: &str) -> Self {
        PartResult { warnings: vec![warning.to_owned()], ..PartResult::new(stem, source) }
    }

    /// The log line the Python converter prints for a part.
    pub fn log_line(&self) -> String {
        let detail = if self.warnings.is_empty() { String::new() } else { format!(" - {}", self.warnings.join("; ")) };
        format!("  {} {} ({} s){}", if self.ok { "OK " } else { "ERR" }, self.stem, crate::util::py_repr(self.seconds), detail)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub source: String,
    pub output: Option<String>,
    pub instances: usize,
    pub placed: usize,
    pub parts: Vec<PartResult>,
    pub warnings: Vec<String>,
    pub seconds: f64,
}

impl Report {
    pub fn new(source: String) -> Self {
        Report { source, ..Default::default() }
    }

    pub fn ok(&self) -> bool {
        self.output.is_some() && self.parts.iter().all(|p| p.ok && p.matches_source != Some(false)) && self.placed == self.instances
    }

    pub fn finished_line(&self) -> String {
        format!(
            "{}: {} ({} s)",
            if self.ok() { "Terminado" } else { "Terminado con avisos" },
            self.output.as_deref().unwrap_or("None"),
            crate::util::py_repr(self.seconds)
        )
    }
}

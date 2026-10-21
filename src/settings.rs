//! Where things are on this computer, and the window's remembered choices. The settings file
//! is the Python window's, so both remember the same choices.

use std::path::PathBuf;

use serde_json::{Map, Value};

pub fn default_out() -> PathBuf {
    home().join("Documents").join("SolidCraft convertidos")
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

pub fn settings_file() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home).join("cad2solidcraft").join("settings.json")
}

/// The SolidCraft desktop app, for opening results.
pub fn find_solidcraft() -> Option<PathBuf> {
    for var in ["ProgramFiles", "LOCALAPPDATA"] {
        let Some(base) = std::env::var_os(var).map(PathBuf::from) else { continue };
        for p in [base.join("SolidCraft").join("solidcraft.exe"), base.join("Programs").join("SolidCraft").join("solidcraft.exe")] {
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

pub fn load() -> Map<String, Value> {
    std::fs::read_to_string(settings_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

pub fn save(data: &Map<String, Value>) {
    let path = settings_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = crate::util::write_json(&path, data);
}

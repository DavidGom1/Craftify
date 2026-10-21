//! Talk to Fusion through the Cad2SolidCraftExporter add-in. Port of `fusion/bridge.py`.
//!
//! Fusion must be installed on this computer and signed in. The add-in (Python, run by Fusion
//! itself) travels inside this program and is copied into Fusion's add-ins folder (it starts
//! with Fusion), Fusion is started when it is not running, and a job folder is handed over; the
//! add-in answers with STEP files and result.json in that folder.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::util;

pub const ADDIN: &str = "Cad2SolidCraftExporter";

/// The add-in's files, built into the program.
const ADDIN_FILES: [(&str, &[u8]); 2] = [
    ("Cad2SolidCraftExporter.py", include_bytes!("../../fusion_addin/Cad2SolidCraftExporter/Cad2SolidCraftExporter.py")),
    (
        "Cad2SolidCraftExporter.manifest",
        include_bytes!("../../fusion_addin/Cad2SolidCraftExporter/Cad2SolidCraftExporter.manifest"),
    ),
];

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(home)
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default()
}

pub fn root_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join("cad2solidcraft").join("fusion")
}

fn jobs() -> PathBuf {
    root_dir().join("jobs")
}

fn alive_file() -> PathBuf {
    root_dir().join("alive.json")
}

pub fn addins_dir() -> PathBuf {
    env_dir("APPDATA").join("Autodesk").join("Autodesk Fusion 360").join("API").join("AddIns")
}

/// Copy the add-in into Fusion's add-ins folder; true when it changed (Fusion must restart).
pub fn install_addin() -> Result<bool> {
    let dst = addins_dir().join(ADDIN);
    std::fs::create_dir_all(&dst)?;
    let mut changed = false;
    for (name, bytes) in ADDIN_FILES {
        let target = dst.join(name);
        if std::fs::read(&target).ok().as_deref() != Some(bytes) {
            std::fs::write(&target, bytes)?;
            changed = true;
        }
    }
    Ok(changed)
}

pub fn find_fusion() -> Option<PathBuf> {
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Autodesk").join("webdeploy").join("production");
    let mut launchers: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(base)
        .ok()?
        .flatten()
        .map(|e| e.path().join("FusionLauncher.exe"))
        .filter(|p| p.is_file())
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    launchers.sort_by(|a, b| b.0.cmp(&a.0));
    launchers.into_iter().next().map(|(_, p)| p)
}

fn fusion_running() -> bool {
    let mut cmd = Command::new("tasklist");
    cmd.args(["/FI", "IMAGENAME eq Fusion360.exe", "/NH"]);
    util::no_window(&mut cmd);
    cmd.output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("Fusion360.exe"))
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn alive(max_age: f64) -> bool {
    let Ok(text) = std::fs::read_to_string(alive_file()) else { return false };
    let Ok(data) = serde_json::from_str::<Value>(&text) else { return false };
    let t = data.get("time").and_then(Value::as_f64).unwrap_or(0.0);
    now() - t < max_age
}

/// Fusion running with the add-in loaded, or a clear error.
pub fn ensure_ready(log: &dyn Fn(&str), timeout: Duration) -> Result<()> {
    let changed = install_addin()?;
    if alive(6.0) {
        if changed {
            log("El complemento de Fusion se ha actualizado: reinicia Fusion para usar la versión nueva.");
        }
        return Ok(());
    }
    if fusion_running() {
        bail!(
            "Fusion está abierto pero el complemento de exportación no está cargado. \
             Cierra Fusion y vuelve a convertir (se abrirá solo con el complemento), \
             o actívalo en Utilidades > Complementos > Cad2SolidCraftExporter."
        );
    }
    let exe = find_fusion().ok_or_else(|| anyhow!("Fusion no está instalado en este equipo: hace falta para convertir ficheros de Fusion."))?;
    log("Abriendo Fusion…");
    let mut cmd = Command::new(&exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(DETACHED_PROCESS);
    }
    cmd.spawn().map_err(|e| anyhow!("{}: {e}", exe.display()))?;
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        if alive(6.0) {
            log(&format!("Fusion listo en {:.0} s.", t0.elapsed().as_secs_f64()));
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    bail!("Fusion no ha cargado el complemento de exportación a tiempo. Comprueba que has iniciado sesión.")
}

fn job_name() -> String {
    // The local time and a short random tag, as the Python bridge names its jobs.
    let secs = now();
    let tag = format!("{:06x}", (secs.fract() * 16_777_216.0) as u32 ^ std::process::id());
    format!("{}-{}", util_time_stamp(), &tag[tag.len() - 6..])
}

/// `time.strftime('%Y%m%d-%H%M%S')` in UTC (the name only has to be unique and sortable).
fn util_time_stamp() -> String {
    let secs = now() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Have Fusion export a design; returns (result.json, the folder with the STEP files).
pub fn export(src: &Path, log: &dyn Fn(&str)) -> Result<(Value, PathBuf)> {
    ensure_ready(log, Duration::from_secs(420))?;
    let job = jobs().join(job_name());
    std::fs::create_dir_all(&job)?;
    let tmp = job.join("job.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&json!({ "source": util::s(src) }))?)?;
    std::fs::rename(&tmp, job.join("job.json"))?;
    log("Exportando desde Fusion…");
    let t0 = Instant::now();
    let result = job.join("result.json");
    while !result.is_file() {
        if t0.elapsed() > Duration::from_secs(1800) {
            bail!("Fusion no ha terminado la exportación a tiempo.");
        }
        if !alive(30.0) {
            bail!("Fusion se ha cerrado durante la exportación.");
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    let data: Value = serde_json::from_str(&std::fs::read_to_string(&result)?)?;
    if !data.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let error = data.get("error").and_then(Value::as_str).unwrap_or("");
        bail!("Fusion no ha podido exportar el diseño:\n{}", error.trim().lines().last().unwrap_or(""));
    }
    let count = |k: &str| data.get(k).and_then(Value::as_array).map_or(0, Vec::len);
    log(&format!(
        "Exportado desde Fusion en {:.0} s: {} componentes, {} ocurrencias.",
        t0.elapsed().as_secs_f64(),
        count("components"),
        count("occurrences")
    ));
    Ok((data, job))
}

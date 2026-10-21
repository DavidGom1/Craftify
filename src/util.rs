//! Small helpers that keep the Rust converter's output identical to the Python one.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};

/// A float as Python's `repr` writes it (shortest round trip, `1e-05`, `2.0`).
pub fn py_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    // Rust's `{:e}` gives the same shortest digits Python chooses.
    let sci = format!("{x:e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let decpt = exp + 1; // digits are 0.d1d2... x 10^decpt
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if decpt <= -4 || decpt > 16 {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exp.abs()));
    } else if decpt <= 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat((-decpt) as usize));
        out.push_str(&digits);
    } else if (decpt as usize) < digits.len() {
        out.push_str(&digits[..decpt as usize]);
        out.push('.');
        out.push_str(&digits[decpt as usize..]);
    } else {
        out.push_str(&digits);
        out.push_str(&"0".repeat(decpt as usize - digits.len()));
        out.push_str(".0");
    }
    out
}

/// Python's `sum()` of floats: since Python 3.12 it compensates rounding (Neumaier), so a plain
/// running total differs in the last digits.
pub fn py_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let (mut f, mut c) = (0.0_f64, 0.0_f64);
    for x in values {
        let t = f + x;
        if f.abs() >= x.abs() {
            c += (f - t) + x;
        } else {
            c += (x - t) + f;
        }
        f = t;
    }
    if c != 0.0 && c.is_finite() {
        f += c;
    }
    f
}

/// Python's `round(x, 1)` for the seconds a step took.
pub fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// A text file read as Python's `read_text(encoding="latin-1")` does: one char per byte and
/// universal newlines.
pub fn read_latin1(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\r' {
            out.push('\n');
            if bytes.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
        } else {
            out.push(b as char);
        }
        i += 1;
    }
    Ok(out)
}

/// The counterpart of [`read_latin1`]. Characters past U+00FF cannot be written, as in Python.
pub fn write_latin1(path: &Path, text: &str) -> Result<()> {
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let c = ch as u32;
        if c > 0xFF {
            return Err(anyhow!("'latin-1' codec can't encode character {ch:?}"));
        }
        bytes.push(c as u8);
    }
    std::fs::write(path, bytes).map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// A path with the separators Windows uses, as `pathlib` gives them.
pub fn native(path: &Path) -> PathBuf {
    if cfg!(windows) { PathBuf::from(path.to_string_lossy().replace('/', "\\")) } else { path.to_path_buf() }
}

/// `str(path)`.
pub fn s(path: &Path) -> String {
    native(path).to_string_lossy().into_owned()
}

/// The file name without its last extension (`Path.stem`).
pub fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The lower-case extension with its dot (`Path.suffix.lower()`).
pub fn suffix_lower(path: &Path) -> String {
    path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default()
}

/// `shutil.which`: a program on the PATH.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.BAT;.CMD".into()).split(';').map(str::to_owned).collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// The folder of this executable.
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

pub struct Output {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug)]
pub struct TimedOut(pub String);

impl std::fmt::Display for TimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Command '{}' timed out", self.0)
    }
}

impl std::error::Error for TimedOut {}

/// A child program without a console window of its own, its output collected, killed after
/// `timeout` (Python's `subprocess.run(..., timeout=...)`).
pub fn run(program: &Path, args: &[&str], timeout: Duration) -> Result<Output> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| anyhow!("{}: {e}", program.display()))?;
    let mut out = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let mut err = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;
    let t_out = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out.read_to_end(&mut v);
        v
    });
    let t_err = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err.read_to_end(&mut v);
        v
    });
    let t0 = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if t0.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(TimedOut(format!("{} {}", program.display(), args.join(" "))).into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = String::from_utf8_lossy(&t_out.join().unwrap_or_default()).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&t_err.join().unwrap_or_default()).replace("\r\n", "\n");
    Ok(Output { status: status.code(), stdout, stderr })
}

#[cfg(windows)]
pub fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn no_window(_cmd: &mut Command) {}

/// Write JSON as Python's `json.dumps(indent=1, ensure_ascii=False)` lays it out.
pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    value.serialize(&mut ser)?;
    std::fs::write(path, buf).map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// The last non-empty line of a program's output, for an error message.
pub fn last_line(text: &str) -> Option<String> {
    text.trim().lines().last().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::py_repr;

    #[test]
    fn repr_matches_python() {
        for (x, want) in [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (0.1, "0.1"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (1.5e-7, "1.5e-07"),
            (123.456, "123.456"),
            (1e16, "1e+16"),
            (1234567890123456.0, "1234567890123456.0"),
            (-2.5e22, "-2.5e+22"),
            (6.123233995736766e-17, "6.123233995736766e-17"),
            (100.0, "100.0"),
        ] {
            assert_eq!(py_repr(x), want);
        }
    }
}

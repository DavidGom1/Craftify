//! Read the component instances of a SolidWorks assembly (.SLDASM). Port of `sw/assembly.py`.
//!
//! Observed layout of the `Contents/Config-0` block, from the user's own files: strings are
//! MFC Unicode strings (`FF FE FF`, a length, UTF-16LE characters). Each placed component is a
//! record that starts with its instance name (`<file stem>-<n>`). 145 bytes after the name ends
//! comes its placement: either 13 little-endian doubles (a 3x3 rotation, the translation in
//! metres and the scale) or, when the component is not rotated, only the translation and the
//! scale. The name of the referenced configuration follows the placement.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use unicode_general_category::{GeneralCategory as G, get_general_category};

use super::container::{block, read_file};
use crate::util::py_sum;

const PLACEMENT_GAP: usize = 145;
const FULL_TO_CONFIG: usize = 196; // placement start -> configuration string, rotated components
const SHORT_TO_CONFIG: usize = 124; // placement start -> configuration string, translation only
pub const IDENTITY: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

static INSTANCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?P<stem>.+)-(?P<n>\d+)$").expect("instance pattern"));

pub struct CString {
    /// Offset of the FF FE FF prefix.
    pub start: usize,
    /// Offset just past the last character.
    pub end: usize,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Instance {
    /// SolidWorks instance name, e.g. "FLA 2.10-3".
    pub name: String,
    /// The referenced file's name without extension, e.g. "FLA 2.10".
    pub stem: String,
    /// 3x3, row-major, as stored.
    pub rotation: [f64; 9],
    pub translation_mm: [f64; 3],
    pub configuration: Option<String>,
    pub rotated: bool,
}

pub struct Assembly {
    pub path: PathBuf,
    pub name: String,
    pub instances: Vec<Instance>,
    /// Every file path the assembly names.
    pub references: Vec<String>,
    pub warnings: Vec<String>,
}

/// Python's `str.isprintable` for one character.
fn printable(ch: char) -> bool {
    if ch == ' ' {
        return true;
    }
    !matches!(
        get_general_category(ch),
        G::Control
            | G::Format
            | G::Surrogate
            | G::PrivateUse
            | G::Unassigned
            | G::LineSeparator
            | G::ParagraphSeparator
            | G::SpaceSeparator
    )
}

pub fn read_cstrings(buf: &[u8]) -> Vec<CString> {
    let mut out = Vec::new();
    let mut pos = 0;
    loop {
        let Some(i) = (pos < buf.len()).then(|| buf[pos..].windows(3).position(|w| w == b"\xff\xfe\xff")).flatten() else {
            return out;
        };
        let i = i + pos;
        if i + 4 > buf.len() {
            return out;
        }
        let mut n = buf[i + 3] as usize;
        let mut start = i + 4;
        if n == 0xFF {
            // long string: u16 length follows
            if i + 6 > buf.len() {
                return out;
            }
            n = u16::from_le_bytes([buf[i + 4], buf[i + 5]]) as usize;
            start = i + 6;
        }
        let end = start + 2 * n;
        if n == 0 || end > buf.len() {
            pos = i + 1;
            continue;
        }
        let units: Vec<u16> = buf[start..end].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        match String::from_utf16(&units) {
            Ok(text) if text.chars().all(printable) => {
                out.push(CString { start: i, end, text });
                pos = end;
            }
            _ => pos = i + 1,
        }
    }
}

fn is_rotation(m: &[f64]) -> bool {
    let tol = 1e-6;
    let rows = [&m[0..3], &m[3..6], &m[6..9]];
    if rows.iter().any(|r| (py_sum(r.iter().map(|x| x * x)) - 1.0).abs() > tol) {
        return false;
    }
    for i in 0..3 {
        for j in i + 1..3 {
            let d = py_sum((0..3).map(|k| rows[i][k] * rows[j][k]));
            if d.abs() > tol {
                return false;
            }
        }
    }
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6]);
    (det - 1.0).abs() < tol
}

fn doubles(buf: &[u8], at: usize, n: usize) -> Vec<f64> {
    (0..n)
        .map(|k| {
            let s = at + 8 * k;
            f64::from_le_bytes(buf[s..s + 8].try_into().unwrap_or([0; 8]))
        })
        .collect()
}

/// (rotation, translation in metres, configuration, rotated).
fn placement(buf: &[u8], at: usize, by_start: &HashMap<usize, &CString>) -> Option<([f64; 9], [f64; 3], String, bool)> {
    if at + 13 * 8 <= buf.len() {
        let v = doubles(buf, at, 13);
        if v.iter().all(|x| x.is_finite()) && is_rotation(&v[..9]) && (v[12] - 1.0).abs() < 1e-9
            && let Some(cfg) = by_start.get(&(at + FULL_TO_CONFIG)) {
                let mut r = [0.0; 9];
                r.copy_from_slice(&v[..9]);
                return Some((r, [v[9], v[10], v[11]], cfg.text.clone(), true));
            }
    }
    if at + 4 * 8 <= buf.len() {
        let v = doubles(buf, at, 4);
        if v.iter().all(|x| x.is_finite()) && (v[3] - 1.0).abs() < 1e-9 && v[..3].iter().all(|x| x.abs() < 1e4)
            && let Some(cfg) = by_start.get(&(at + SHORT_TO_CONFIG)) {
                return Some((IDENTITY, [v[0], v[1], v[2]], cfg.text.clone(), false));
            }
    }
    None
}

pub fn parse_instances(config_block: &[u8]) -> Vec<Instance> {
    let strings = read_cstrings(config_block);
    let by_start: HashMap<usize, &CString> = strings.iter().map(|s| (s.start, s)).collect();
    let mut found = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for s in &strings {
        let Some(m) = INSTANCE.captures(&s.text) else { continue };
        if seen.contains(&s.text) {
            continue;
        }
        let Some((rotation, t_m, configuration, rotated)) = placement(config_block, s.end + PLACEMENT_GAP, &by_start) else {
            continue;
        };
        seen.insert(s.text.clone());
        found.push(Instance {
            name: s.text.clone(),
            stem: m["stem"].to_owned(),
            rotation,
            translation_mm: [t_m[0] * 1000.0, t_m[1] * 1000.0, t_m[2] * 1000.0],
            configuration: Some(configuration),
            rotated,
        });
    }
    found
}

pub fn read_assembly(path: &Path) -> Result<Assembly> {
    let blocks = read_file(path)?;
    let mut asm = Assembly {
        path: path.to_path_buf(),
        name: crate::util::stem(path),
        instances: Vec::new(),
        references: Vec::new(),
        warnings: Vec::new(),
    };
    let Some(config) = block(&blocks, "Contents/Config-0") else {
        asm.warnings.push("no readable Contents/Config-0 block: the component list could not be read".into());
        return Ok(asm);
    };
    asm.instances = parse_instances(config);
    if let Some(header) = block(&blocks, "Header2") {
        asm.references = read_cstrings(header)
            .into_iter()
            .map(|s| s.text)
            .filter(|t| {
                let low = t.to_lowercase();
                low.ends_with(".sldprt") || low.ends_with(".sldasm")
            })
            .collect();
    }
    Ok(asm)
}

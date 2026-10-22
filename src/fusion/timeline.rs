//! A component's timeline, read from the Design segment (see [`super::f3d`]): the features in
//! order, with what is needed to make them again in SolidCraft.
//!
//! Each component has a feature list (`3A6D1E62`) that points at its features in timeline order.
//! A feature's parameters are value records (`D91D429C`: the value in internal units at byte
//! 40) each after a record (`7A6A3D31`) with its expression and name ("85.6 mm", "DistanceX").
//! What is read:
//!
//! * Box primitive (`F193B033`): DistanceX/Y/Z and OffsetDistanceX/Y, and a rigid 4x4 that places
//!   it.
//! * Fillet (`A07D5F17`): its radius. Fusion names the edges by its topological naming (an
//!   "edge_recipe_data" record), which is not read: the edges are found again from the finished
//!   part (see `offline.rs`).
//! * Extrude (`DD405BC2`): the operation (a u32 at byte 28: 2 cut, 4 new body), the distance and
//!   the taper. The profiles of a sketch it extrudes are listed in a profile table (`0D57BD2F`,
//!   reached through a `4BD53E5A` that points at the extrude): per profile its loops, outer one
//!   first, each a list of the keys of its sketch curves. An extrude without a table extruded
//!   faces of the model (named by topology, not read).
//! * Sketches (`8DA771B7`): read by [`super::sketch`].
//!
//! Anything else in a component's list that carries a timeline name stops the rebuild there.

use super::f3d::{Mat4, Segment, find_matrix};
use super::sketch::{f64_at, is, refs_to, u32_at, u64_at};
use std::collections::HashMap;

const FEATURES: &str = "3A6D1E62-99F8-4E32-BAAA-DBA0A2CFB27C"; // a component's feature list
const SKETCH: &str = "8DA771B7-52ED-42FC-94E2-2353FE141373";
const BOX: &str = "F193B033-8CB7-4CC9-B18E-D1E06A07D72E";
const FILLET: &str = "A07D5F17-68CB-464D-9935-BF68E98A865F";
const EXTRUDE: &str = "DD405BC2-D673-44F0-8833-5CB2A1C186C7";
const VALUE: &str = "D91D429C-1BB5-4640-B49F-69AF14F463E9"; // a parameter's value
const PARAMETER: &str = "7A6A3D31-BE74-4E19-9115-20642944C2E4"; // its expression and name
const PROFILE_SET: &str = "4BD53E5A-0B3E-45E5-AE8B-02044306485A"; // points at the extrude and its table
const PROFILE_TABLE: &str = "0D57BD2F-D09B-43FC-AD57-1E89A118C453";

/// A parameter: its expression as Fusion shows it and its value (mm for lengths, radians for
/// angles).
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub expression: String,
    pub value: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Cut,
    NewBody,
}

/// A profile of a sketch: its outer loop and its holes, each the keys of its curves.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub outer: Vec<u64>,
    pub holes: Vec<Vec<u64>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Feature {
    Box {
        /// DistanceX, DistanceY, DistanceZ in the box's own axes.
        size: [Param; 3],
        /// OffsetDistanceX/Y: where in its plane the box starts, mm.
        offset: [f64; 2],
        /// Box axes to the component's: row-major, translation in mm.
        placement: Box<Mat4>,
    },
    Fillet {
        radius: Param,
    },
    Sketch {
        id: u64,
    },
    Extrude {
        operation: Operation,
        distance: Param,
        /// The sketch profiles it extrudes; None when it extruded faces of the model.
        profiles: Option<Vec<Profile>>,
    },
    /// A feature not read yet: its type and the name Fusion shows.
    Other {
        kind: String,
        name: String,
    },
}

/// A length-prefixed UTF-16 string of printable ASCII at `at`, and where it ends.
fn lp_text(rec: &[u8], at: usize) -> Option<(String, usize)> {
    let n = u32_at(rec, at)? as usize;
    if !(1..128).contains(&n) {
        return None;
    }
    let bytes = rec.get(at + 4..at + 4 + 2 * n)?;
    let t: Option<String> =
        bytes.as_chunks::<2>().0.iter().map(|c| (c[1] == 0 && (0x20..0x7f).contains(&c[0])).then_some(c[0] as char)).collect();
    Some((t?, at + 4 + 2 * n))
}

/// A parameter record's expression and name: two length-prefixed strings, the expression with a
/// digit in it and, within 32 bytes after it, the name, an identifier ("85.6 mm", "DistanceX").
fn expression_and_name(rec: &[u8]) -> Option<(String, String)> {
    let identifier =
        |n: &str| n.starts_with(|c: char| c.is_ascii_alphabetic()) && n.chars().all(|c| c.is_ascii_alphanumeric() || " _-".contains(c));
    (0..rec.len()).find_map(|i| {
        let (expression, next) = lp_text(rec, i).filter(|(e, _)| e.chars().any(|c| c.is_ascii_digit()))?;
        let name = (next..next + 32).find_map(|j| lp_text(rec, j).map(|(n, _)| n).filter(|n| identifier(n)))?;
        Some((expression, name))
    })
}

/// The parameters a feature points at, by name.
fn params(s: &Segment, rec: &[u8]) -> HashMap<String, Param> {
    let mut out = HashMap::new();
    for (_, v) in refs_to(s, rec, VALUE) {
        let Some(value) = s.record(v).ok().and_then(|r| f64_at(r, 40)) else { continue };
        let Some(p) = v.checked_sub(1).filter(|p| is(s, *p, PARAMETER)) else { continue };
        let Some((expression, name)) = s.record(p).ok().and_then(expression_and_name) else { continue };
        // lengths are kept in centimetres
        let unit = expression.trim_end();
        let value =
            if unit.ends_with("mm") || unit.ends_with("cm") || unit.ends_with(" m") || unit.ends_with("in") { value * 10.0 } else { value };
        out.entry(name.clone()).or_insert(Param { name, expression, value });
    }
    out
}

/// A profile table: per profile its loops (outer first), each the keys of its curves.
fn profile_table(rec: &[u8]) -> Option<Vec<Profile>> {
    let mut p = 32;
    let n = u32_at(rec, p)? as usize;
    p += 4;
    let mut out = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        let loops = u32_at(rec, p)? as usize;
        p += 4;
        let mut outer = None;
        let mut holes = Vec::new();
        for _ in 0..loops {
            let count = u32_at(rec, p)? as usize;
            p += 4;
            let mut keys = Vec::with_capacity(count.min(4096));
            for _ in 0..count {
                keys.push(u64_at(rec, p + 4)?);
                p += 40;
            }
            let is_outer = *rec.get(p)? == 1;
            p += 1;
            if is_outer && outer.is_none() {
                outer = Some(keys);
            } else {
                holes.push(keys);
            }
        }
        p += 4;
        out.push(Profile { outer: outer?, holes });
    }
    (p <= rec.len()).then_some(out)
}

fn extrude(s: &Segment, e: u64, rec: &[u8]) -> Feature {
    let p = params(s, rec);
    let other = |why: &str| Feature::Other { kind: format!("Extruir ({why})"), name: String::new() };
    let operation = match u32_at(rec, 28) {
        Some(2) => Operation::Cut,
        Some(4) => Operation::NewBody,
        _ => return other("operación"),
    };
    let Some(distance) = p.get("AlongDistance").cloned() else { return other("distancia") };
    if p.get("TaperAngle").is_some_and(|t| t.value.abs() > 1e-12) {
        return other("ángulo de salida");
    }
    // its profile table: through a profile set that points at this extrude
    let profiles = s
        .of_type(PROFILE_SET)
        .iter()
        .filter_map(|ps| s.record(*ps).ok().map(|r| (*ps, r)))
        .find(|(_, r)| refs_to(s, r, EXTRUDE).any(|(_, x)| x == e))
        .and_then(|(_, r)| refs_to(s, r, PROFILE_TABLE).next())
        .and_then(|(_, t)| s.record(t).ok())
        .and_then(profile_table);
    Feature::Extrude { operation, distance, profiles }
}

fn box_primitive(s: &Segment, rec: &[u8]) -> Feature {
    let p = params(s, rec);
    let get = |n: &str| p.get(n).cloned();
    match (get("DistanceX"), get("DistanceY"), get("DistanceZ"), find_matrix(rec)) {
        (Some(x), Some(y), Some(z), Some(placement)) => Feature::Box {
            size: [x, y, z],
            offset: [get("OffsetDistanceX").map_or(0.0, |o| o.value), get("OffsetDistanceY").map_or(0.0, |o| o.value)],
            placement: Box::new(placement),
        },
        _ => Feature::Other { kind: "CajaPrimitiva".into(), name: String::new() },
    }
}

/// A timeline name ("Boceto2", "CrearComponentes1"): after ff ff ff ff, a UTF-16 word and its
/// number.
fn timeline_name(rec: &[u8]) -> Option<String> {
    let at = rec.windows(4).position(|w| w == [0xff; 4])? + 4;
    let n = u32_at(rec, at)? as usize;
    if !(1..64).contains(&n) {
        return None;
    }
    let units: Vec<u16> = rec.get(at + 4..at + 4 + 2 * n)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let word = String::from_utf16(&units).ok().filter(|w| w.chars().all(|c| c.is_alphanumeric() || c == ' '))?;
    let num = u32_at(rec, at + 4 + 2 * n).filter(|k| (1..1 << 20).contains(k));
    Some(num.map_or(word.clone(), |k| format!("{word}{k}")))
}

/// Each component's timeline, by component record id (`components` sorted).
pub fn read_timelines(s: &Segment, components: &[u64]) -> HashMap<u64, Vec<Feature>> {
    let mut out = HashMap::new();
    for &list in s.of_type(FEATURES) {
        let Ok(rec) = s.record(list) else { continue };
        let i = components.partition_point(|c| *c <= list);
        if i == 0 {
            continue;
        }
        let mut seen = Vec::new();
        let mut features = Vec::new();
        // flagged references, in the order the list keeps them
        for at in 1..rec.len().saturating_sub(7) {
            if rec[at - 1] != 1 {
                continue;
            }
            let Some(e) = u64_at(rec, at).filter(|e| *e != list && s.index.contains_key(e)) else { continue };
            if seen.contains(&e) {
                continue;
            }
            let Ok(frec) = s.record(e) else { continue };
            let f = if is(s, e, SKETCH) {
                Some(Feature::Sketch { id: e })
            } else if is(s, e, BOX) {
                Some(box_primitive(s, frec))
            } else if is(s, e, FILLET) {
                Some(match params(s, frec).get("Radius") {
                    Some(r) => Feature::Fillet { radius: r.clone() },
                    None => Feature::Other { kind: "Empalme".into(), name: String::new() },
                })
            } else if is(s, e, EXTRUDE) {
                Some(extrude(s, e, frec))
            } else {
                timeline_name(frec).map(|name| Feature::Other { kind: s.guid_of(e).unwrap_or("?").chars().take(8).collect(), name })
            };
            if let Some(f) = f {
                seen.push(e);
                features.push(f);
            }
        }
        out.insert(components[i - 1], features);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: u64) -> Vec<u8> {
        let mut b = 3u32.to_le_bytes().to_vec();
        b.extend(key.to_le_bytes());
        b.extend([0; 28]);
        b
    }

    fn lp16(s: &str) -> Vec<u8> {
        let mut b = (s.len() as u32).to_le_bytes().to_vec();
        b.extend(s.bytes().flat_map(|c| [c, 0]));
        b
    }

    #[test]
    fn finds_a_parameters_expression_and_name() {
        let mut rec = vec![3, 0, 0, 0, 0x57, 0, 0x57, 0];
        rec.extend(lp16("-0.4 mm"));
        rec.extend([0; 9]);
        rec.extend(lp16("AlongDistance"));
        rec.extend(lp16("mm"));
        assert_eq!(expression_and_name(&rec), Some(("-0.4 mm".into(), "AlongDistance".into())));
    }

    #[test]
    fn reads_a_profile_table() {
        let mut rec = vec![0; 32];
        rec.extend(2u32.to_le_bytes()); // two profiles
        // a square
        rec.extend(1u32.to_le_bytes());
        rec.extend(4u32.to_le_bytes());
        for k in [10, 11, 12, 13] {
            rec.extend(item(k));
        }
        rec.push(1);
        rec.extend([0; 4]);
        // a frame: a hole, then its outer loop
        rec.extend(2u32.to_le_bytes());
        rec.extend(1u32.to_le_bytes());
        rec.extend(item(20));
        rec.push(0);
        rec.extend(2u32.to_le_bytes());
        rec.extend(item(21));
        rec.extend(item(22));
        rec.push(1);
        rec.extend([0; 4]);
        let t = profile_table(&rec).expect("a table");
        assert_eq!(t[0], Profile { outer: vec![10, 11, 12, 13], holes: vec![] });
        assert_eq!(t[1], Profile { outer: vec![21, 22], holes: vec![vec![20]] });
    }
}

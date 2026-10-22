//! A Fusion design's sketches, read from the Design segment (see [`super::f3d`]).
//!
//! Each sketch has a container record that points back at it; its points, lines, arcs and
//! splines each point at that container with the last flagged reference of their record. The
//! sketch's plane is a matrix in a record it points at, its name "Boceto" plus a number (or the
//! name given to it, kept in the container) and its component the one whose feature list it
//! points at. Coordinates are centimetres in the sketch's own plane.
//!
//! Worked out against what Fusion's API reports for the same files: every point, line, arc,
//! circle and cubic spline of 42 sketches in two designs.

use std::collections::HashMap;

use serde_json::json;

use super::f3d::{IDENTITY4, Mat4, Segment, find_matrix};
use crate::solidcraft::script::Script;

const SKETCH: &str = "8DA771B7-52ED-42FC-94E2-2353FE141373";
const CONTAINER: &str = "44A64366-4BD3-4B24-881A-F94C206E8F2D"; // what a sketch's curves point at
const PLANE: &str = "7B6C3C2D-4096-4E6D-B225-699E8C2D9355";
const FEATURES: &str = "3A6D1E62-99F8-4E32-BAAA-DBA0A2CFB27C"; // a component's feature list
const POINT: &str = "C2CEDAE7-1716-47C1-B7B1-07B70081D0FB";
const LINE: &str = "DCA267ED-D615-4934-B64F-AD805E8003E2";
const ARC: &str = "F0130424-8B7E-4092-93C9-1CA807482534"; // arcs and circles
const SPLINE: &str = "D82E012F-6DDD-4AED-BDE1-C0F7F9100B9B";

/// A curve in the sketch's plane, mm.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Line {
        a: [f64; 2],
        b: [f64; 2],
        construction: bool,
    },
    /// Counter-clockwise from `start` to `end`.
    Arc {
        center: [f64; 2],
        start: [f64; 2],
        end: [f64; 2],
    },
    Circle {
        center: [f64; 2],
        radius: f64,
    },
    /// A cubic Bézier: four control points.
    Spline {
        points: [[f64; 2]; 4],
    },
}

pub struct Sketch {
    pub id: u64,
    pub name: String,
    /// The plane in the component's coordinates, mm.
    pub origin: [f64; 3],
    pub x_dir: [f64; 3],
    pub y_dir: [f64; 3],
    pub curves: Vec<Curve>,
    /// Each curve's key in the sketch (what a profile lists its curves by), with `curves`.
    pub keys: Vec<u64>,
    /// Curves of kinds not read yet (ellipses, texts, projected edges…).
    pub skipped: usize,
}

/// A curve's key, the number profiles name it by: a u64 at byte 122 of its record.
const KEY_AT: usize = 122;

pub(super) fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

pub(super) fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

pub(super) fn f64_at(b: &[u8], at: usize) -> Option<f64> {
    Some(f64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

pub(super) fn is(s: &Segment, e: u64, guid: &str) -> bool {
    s.guid_of(e).is_some_and(|g| g.eq_ignore_ascii_case(guid))
}

/// Every reference (u64 id of a record of that type) in the record, with its position.
pub(super) fn refs_to<'a>(s: &'a Segment, rec: &'a [u8], guid: &'a str) -> impl Iterator<Item = (usize, u64)> + 'a {
    (0..rec.len().saturating_sub(7)).filter_map(move |i| u64_at(rec, i).filter(|x| is(s, *x, guid)).map(|x| (i, x)))
}

/// The last flagged reference (a byte 1, then the id) to a record of that type.
pub(super) fn last_flagged(s: &Segment, rec: &[u8], guid: &str) -> Option<u64> {
    (1..rec.len().saturating_sub(7)).rev().filter(|&i| rec[i - 1] == 1).filter_map(|i| u64_at(rec, i)).find(|x| is(s, *x, guid))
}

/// A length-prefixed UTF-16 string that reads as a name.
fn name_at(rec: &[u8], at: usize) -> Option<String> {
    let n = u32_at(rec, at)? as usize;
    if !(1..128).contains(&n) {
        return None;
    }
    let units: Vec<u16> = rec.get(at + 4..at + 4 + 2 * n)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let t = String::from_utf16(&units).ok()?;
    let ok = t.chars().all(|c| !c.is_control() && (c as u32) < 0x250) && t.chars().any(char::is_alphabetic) && t.trim() == t;
    ok.then_some(t)
}

/// Fusion's default name: after the bytes ff ff ff ff, the word and then its number.
fn default_name(rec: &[u8]) -> Option<String> {
    let at = rec.windows(4).position(|w| w == [0xff; 4])? + 4;
    let word = name_at(rec, at)?;
    let n = u32_at(rec, at + 4 + 2 * word.encode_utf16().count())?;
    Some(if (1..1 << 20).contains(&n) { format!("{word}{n}") } else { word })
}

/// The name the user gave the sketch: in its container, shortly after the reference to it.
fn own_name(container: &[u8], back: usize) -> Option<String> {
    (back + 8..(back + 78).min(container.len())).find_map(|i| name_at(container, i))
}

fn point(s: &Segment, e: u64, points: &mut HashMap<u64, Option<[f64; 2]>>) -> Option<[f64; 2]> {
    *points.entry(e).or_insert_with(|| {
        let rec = s.record(e).ok()?;
        Some([f64_at(rec, 141)? * 10.0, f64_at(rec, 149)? * 10.0])
    })
}

fn finite(p: &[f64]) -> bool {
    p.iter().all(|x| x.is_finite() && x.abs() < 1e7)
}

/// A spline's arrays: degree, tolerance and three arrays (knots, weights, control points),
/// each after its count twice and the size of a number. None: no such header at `i`.
fn nurbs_at(rec: &[u8], i: usize) -> Option<(u32, Vec<f64>, Vec<[f64; 2]>)> {
    let (deg, tol, nk) = (u32_at(rec, i)?, f64_at(rec, i + 4)?, u32_at(rec, i + 12)? as usize);
    let array = |at: usize, n: usize| u32_at(rec, at + 4) == Some(n as u32) && u32_at(rec, at + 8) == Some(8);
    if !(1..=9).contains(&deg) || !(tol > 0.0 && tol < 1e-3) || !(2..4096).contains(&nk) || !array(i + 12, nk) {
        return None;
    }
    let p = i + 24 + 8 * nk;
    let nw = u32_at(rec, p)? as usize;
    if nw >= 4096 || !array(p, nw) {
        return None;
    }
    let q = p + 12 + 8 * nw;
    let nc = u32_at(rec, q)? as usize;
    if nc >= 4096 || !array(q, nc) {
        return None;
    }
    let weights = (0..nw).map(|k| f64_at(rec, p + 12 + 8 * k)).collect::<Option<Vec<f64>>>()?;
    let points =
        (0..nc).map(|k| Some([f64_at(rec, q + 12 + 24 * k)? * 10.0, f64_at(rec, q + 20 + 24 * k)? * 10.0])).collect::<Option<_>>()?;
    Some((deg, weights, points))
}

/// The spline as a cubic Bézier, when it is one: cubic, four control points, not rational.
fn bezier(rec: &[u8]) -> Option<[[f64; 2]; 4]> {
    let (deg, weights, points) = (150..rec.len().saturating_sub(24)).find_map(|i| nurbs_at(rec, i))?;
    if deg != 3 || weights.iter().any(|w| (w - 1.0).abs() > 1e-12) {
        return None;
    }
    points.try_into().ok()
}

/// The sketches of each component (by component record id; `components` sorted).
pub fn read_sketches(s: &Segment, components: &[u64]) -> HashMap<u64, Vec<Sketch>> {
    let mut by_id: HashMap<u64, Sketch> = HashMap::new();
    let mut component_of: HashMap<u64, u64> = HashMap::new();
    for &e in s.of_type(SKETCH) {
        let Ok(rec) = s.record(e) else { continue };
        let planes: Vec<u64> = refs_to(s, rec, PLANE).map(|(_, p)| p).collect();
        if planes.is_empty() {
            continue;
        }
        // a sketch on the origin's XY plane keeps no matrix
        let m = planes.iter().find_map(|p| s.record(*p).ok().and_then(find_matrix)).unwrap_or(IDENTITY4);
        let anchor = last_flagged(s, rec, FEATURES).unwrap_or(e);
        let i = components.partition_point(|c| *c <= anchor);
        if i == 0 {
            continue;
        }
        component_of.insert(e, components[i - 1]);
        let plane = |m: &Mat4| ([m[3], m[7], m[11]], [m[0], m[4], m[8]], [m[1], m[5], m[9]]);
        let (origin, x_dir, y_dir) = plane(&m);
        let name = default_name(rec).unwrap_or_else(|| "Boceto".into());
        by_id.insert(e, Sketch { id: e, name, origin, x_dir, y_dir, curves: Vec::new(), keys: Vec::new(), skipped: 0 });
    }
    // container -> its sketch, and the name the user gave it
    let mut owner: HashMap<u64, u64> = HashMap::new();
    for &c in s.of_type(CONTAINER) {
        let Ok(rec) = s.record(c) else { continue };
        let back: Vec<(usize, u64)> = refs_to(s, rec, SKETCH).collect();
        if let [(at, sk)] = back[..]
            && let Some(sketch) = by_id.get_mut(&sk)
        {
            owner.insert(c, sk);
            if let Some(n) = own_name(rec, at) {
                sketch.name = n;
            }
        }
    }
    let mut points: HashMap<u64, Option<[f64; 2]>> = HashMap::new();
    let add = |sk: Option<&u64>, curve: Option<Curve>, rec: &[u8], by_id: &mut HashMap<u64, Sketch>| {
        if let Some(sketch) = sk.and_then(|k| by_id.get_mut(k)) {
            match curve {
                Some(c) => {
                    sketch.curves.push(c);
                    sketch.keys.push(u64_at(rec, KEY_AT).unwrap_or(u64::MAX));
                }
                None => sketch.skipped += 1,
            }
        }
    };
    for &e in s.of_type(LINE) {
        let Ok(rec) = s.record(e) else { continue };
        let sk = last_flagged(s, rec, CONTAINER).and_then(|c| owner.get(&c));
        let pts = (u64_at(rec, 282).filter(|p| is(s, *p, POINT)), u64_at(rec, 293).filter(|p| is(s, *p, POINT)));
        let curve = match pts {
            (Some(a), Some(b)) => point(s, a, &mut points)
                .zip(point(s, b, &mut points))
                .filter(|(a, b)| finite(a) && finite(b) && a != b)
                .map(|(a, b)| Curve::Line { a, b, construction: rec.get(310) == Some(&1) }),
            _ => None,
        };
        add(sk, curve, rec, &mut by_id);
    }
    for &e in s.of_type(ARC) {
        let Ok(rec) = s.record(e) else { continue };
        let sk = last_flagged(s, rec, CONTAINER).and_then(|c| owner.get(&c));
        let center = u64_at(rec, 282).filter(|p| is(s, *p, POINT)).and_then(|p| point(s, p, &mut points));
        let radius = f64_at(rec, 257).map(|r| r * 10.0).filter(|r| *r > 0.0 && r.is_finite());
        let sweep = f64_at(rec, 273).unwrap_or(0.0);
        let curve = match (center, radius) {
            (Some(c), Some(r)) if (sweep - std::f64::consts::TAU).abs() < 1e-9 => Some(Curve::Circle { center: c, radius: r }),
            (Some(c), Some(_)) => {
                let end = u64_at(rec, 293).filter(|p| is(s, *p, POINT)).and_then(|p| point(s, p, &mut points));
                let start = u64_at(rec, 304).filter(|p| is(s, *p, POINT)).and_then(|p| point(s, p, &mut points));
                start.zip(end).map(|(start, end)| Curve::Arc { center: c, start, end })
            }
            _ => None,
        };
        add(
            sk,
            curve.filter(|c| !matches!(c, Curve::Arc { center, .. } | Curve::Circle { center, .. } if !finite(center))),
            rec,
            &mut by_id,
        );
    }
    for &e in s.of_type(SPLINE) {
        let Ok(rec) = s.record(e) else { continue };
        let sk = last_flagged(s, rec, CONTAINER).and_then(|c| owner.get(&c));
        let curve = bezier(rec).filter(|p| p.iter().all(|q| finite(q))).map(|points| Curve::Spline { points });
        add(sk, curve, rec, &mut by_id);
    }
    let mut out: HashMap<u64, Vec<Sketch>> = HashMap::new();
    let mut ids: Vec<u64> = by_id.keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        if let (Some(sk), Some(c)) = (by_id.remove(&id), component_of.get(&id)) {
            out.entry(*c).or_default().push(sk);
        }
    }
    out
}

/// The id SolidCraft gives each curve that [`add_to`] draws: a letter for its kind and the count
/// of that kind in the sketch (l1, a1, c1, s1…).
pub fn curve_ids(sketch: &Sketch) -> Vec<String> {
    let (mut l, mut a, mut c, mut s) = (0, 0, 0, 0);
    sketch
        .curves
        .iter()
        .map(|curve| match curve {
            Curve::Line { .. } => {
                l += 1;
                format!("l{l}")
            }
            Curve::Arc { .. } => {
                a += 1;
                format!("a{a}")
            }
            Curve::Circle { .. } => {
                c += 1;
                format!("c{c}")
            }
            Curve::Spline { .. } => {
                s += 1;
                format!("s{s}")
            }
        })
        .collect()
}

/// SolidCraft refuses a spline with two equal control points in a row (Fusion keeps them: a
/// tangent of zero length): part them by a ten-thousandth of a millimetre along the chord.
fn part_equal_points(mut p: [[f64; 2]; 4]) -> [[f64; 2]; 4] {
    let (dx, dy) = (p[3][0] - p[0][0], p[3][1] - p[0][1]);
    let len = dx.hypot(dy);
    let (ux, uy) = if len > 0.0 { (dx / len, dy / len) } else { (1.0, 0.0) };
    for i in 0..3 {
        if (p[i][0] - p[i + 1][0]).abs() < 1e-9 && (p[i][1] - p[i + 1][1]).abs() < 1e-9 {
            p[i] = [p[i][0] - 1e-4 * ux, p[i][1] - 1e-4 * uy];
            p[i + 1] = [p[i + 1][0] + 1e-4 * ux, p[i + 1][1] + 1e-4 * uy];
        }
    }
    p
}

/// The commands that draw the sketch in SolidCraft. Its curves are not joined where they meet
/// (`connect: false`): Fusion keeps no constraints on them, and joining them puts hundreds of
/// coincidence marks on a sketch such as a QR code. The profiles are the same.
pub fn add_to(script: &mut Script, sketch: &Sketch) {
    script.add(
        "sketch.create",
        json!({"plane": {"origin": sketch.origin, "x_dir": sketch.x_dir, "y_dir": sketch.y_dir}, "name": sketch.name}),
    );
    for c in &sketch.curves {
        match c {
            Curve::Line { a, b, construction } => {
                script.add("sketch.line", json!({"points": [a, b], "construction": construction, "connect": false}));
            }
            Curve::Arc { center, start, end } => {
                script.add("sketch.arc.center_point", json!({"center": center, "start": start, "end": end, "connect": false}));
            }
            Curve::Circle { center, radius } => {
                script.add("sketch.circle.center", json!({"center": center, "radius": radius, "connect": false}));
            }
            Curve::Spline { points } => {
                script.add("sketch.spline.control_point", json!({"points": part_equal_points(*points), "connect": false}));
            }
        }
    }
    script.add("sketch.finish", json!({}));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lp16(s: &str) -> Vec<u8> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut b = (units.len() as u32).to_le_bytes().to_vec();
        b.extend(units.iter().flat_map(|u| u.to_le_bytes()));
        b
    }

    #[test]
    fn default_names_are_the_word_and_its_number() {
        let mut rec = vec![0; 30];
        rec.extend([0xff; 4]);
        rec.extend(lp16("Boceto"));
        rec.extend(4u32.to_le_bytes());
        rec.extend([0; 12]);
        assert_eq!(default_name(&rec).as_deref(), Some("Boceto4"));
    }

    #[test]
    fn a_name_is_latin_text() {
        let mut rec = lp16("Título");
        rec.extend([0; 4]);
        assert_eq!(name_at(&rec, 0).as_deref(), Some("Título"));
        assert_eq!(name_at(&lp16("\u{3633}\u{7b37}2"), 0), None);
        assert_eq!(name_at(&lp16(" x"), 0), None);
    }

    #[test]
    fn reads_a_cubic_bezier() {
        let mut rec = vec![0; 160];
        rec.extend(3u32.to_le_bytes());
        rec.extend(1e-9f64.to_le_bytes());
        for n in [8u32, 8, 8] {
            rec.extend(n.to_le_bytes());
        }
        for k in [0.0f64, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0] {
            rec.extend(k.to_le_bytes());
        }
        for n in [4u32, 4, 8] {
            rec.extend(n.to_le_bytes());
        }
        for _ in 0..4 {
            rec.extend(1.0f64.to_le_bytes());
        }
        for n in [4u32, 4, 8] {
            rec.extend(n.to_le_bytes());
        }
        for (x, y) in [(0.0f64, 0.0f64), (0.1, 0.2), (0.3, 0.2), (0.4, 0.0)] {
            rec.extend(x.to_le_bytes());
            rec.extend(y.to_le_bytes());
            rec.extend(0.0f64.to_le_bytes());
        }
        rec.extend([0; 16]);
        let p = bezier(&rec).expect("a spline");
        assert!((p[1][0] - 1.0).abs() < 1e-12 && (p[2][1] - 2.0).abs() < 1e-12 && (p[3][0] - 4.0).abs() < 1e-12);
    }

    #[test]
    fn equal_control_points_are_parted() {
        let p = part_equal_points([[0.0, 0.0], [1.0, 1.0], [1.0, 1.0], [2.0, 0.0]]);
        assert!(p[1][0] < 1.0 && p[2][0] > 1.0);
        assert!((p[2][0] - p[1][0] - 2e-4).abs() < 1e-12);
        assert_eq!(p[0], [0.0, 0.0]);
    }
}

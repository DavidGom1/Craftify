//! Repair face orientation flags (ADVANCED_FACE same_sense) in a STEP file. Port of `step_fix`.
//!
//! Some converters write every face with the same `same_sense` flag. Where the loops can be
//! trusted they still carry the true orientation: ISO 10303-42 keeps a face on the left of its
//! bounding loops when seen from the face's outward normal.
//!
//! * Planes: the vector area of the loops points along the outward normal. Loops made of one
//!   full circle are left out (in the files this repairs their direction is not reliable).
//! * Cylinders, cones, spheres and tori: along trusted edges (not full circles, not seams),
//!   `normal x tangent` points into the face when the surface normal is the outward one.
//! * Faces still open are settled from resolved neighbours across circular edges: a cylinder or
//!   cone that runs into the material of the plane it starts on is a hole, a coaxial cylinder or
//!   cone continuing it is hollow or solid alike, and a plane capping a boss faces away from it.
//!
//! Faces on surfaces this module cannot evaluate (B-splines, offsets...) are left as they are.

use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;
use indexmap::IndexMap;
use regex::{Captures, Regex};

use super::{Entity, REVOLVED, Step, V, add, cross, dot, mean, mul, norm, sub, unit};
use crate::util::{py_repr, read_latin1, write_latin1};

pub struct Segment {
    pub edge: i64,
    /// A full circle: starts and ends on one vertex.
    pub closed: bool,
    /// In travel order (the path along an arc is not trusted).
    pub points: Vec<V>,
    pub line: bool,
}

impl Segment {
    fn start(&self) -> V {
        self.points[0]
    }
}

fn loop_segments(st: &Step, loop_id: i64, bound_sense: bool) -> Result<Option<Vec<Segment>>> {
    let Some(lp) = st.get(loop_id) else { return Ok(None) };
    if lp.ty != "EDGE_LOOP" {
        return Ok(None);
    }
    let mut segs = Vec::new();
    for oe_ref in lp.a(1)?.list()? {
        let oe = st.e(oe_ref.r()?)?; // ORIENTED_EDGE('', *, *, edge, orientation)
        let ec_id = oe.a(3)?.r()?;
        let ec = st.e(ec_id)?; // EDGE_CURVE('', v1, v2, curve, same_sense)
        let closed = ec.a(1)? == ec.a(2)?;
        let v1 = st.vertex(ec.a(1)?.r()?)?;
        let v2 = st.vertex(ec.a(2)?.r()?)?;
        let forward = ec.a(4)?.is(".T.");
        let curve = ec.a(3)?.r()?;
        let Some(mut pts) = st.curve_samples(curve, if forward { v1 } else { v2 }, if forward { v2 } else { v1 }, closed)? else {
            return Ok(None);
        };
        if !forward {
            pts.reverse(); // now from v1 to v2
        }
        if !oe.a(4)?.is(".T.") {
            pts.reverse();
        }
        let c = st.curve(curve)?;
        segs.push(Segment { edge: ec_id, closed, points: pts, line: c.is_some_and(|c| c.ty == "LINE") });
    }
    if !bound_sense {
        segs.reverse();
        for s in &mut segs {
            s.points.reverse();
        }
    }
    Ok(Some(segs))
}

pub fn face_loops(st: &Step, face: &Entity) -> Result<Option<Vec<Vec<Segment>>>> {
    let mut loops = Vec::new();
    for b in face.a(1)?.list()? {
        let bound = st.e(b.r()?)?;
        let Some(segs) = loop_segments(st, bound.a(1)?.r()?, bound.a(2)?.is(".T."))? else { return Ok(None) };
        loops.push(segs);
    }
    Ok(Some(loops))
}

pub fn edges_of(st: &Step, face: &Entity) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    for b in face.a(1)?.list()? {
        let lp = st.e(st.e(b.r()?)?.a(1)?.r()?)?;
        for oe in lp.a(1)?.list()? {
            out.push(st.e(oe.r()?)?.a(3)?.r()?);
        }
    }
    Ok(out)
}

/// The loops' vertices: the arcs' paths are not trusted.
pub fn face_points(st: &Step, face: &Entity) -> Result<Vec<V>> {
    let loops = face_loops(st, face)?.unwrap_or_default();
    Ok(loops.iter().flat_map(|segs| segs.iter().map(Segment::start)).collect())
}

fn sense_from_loops(st: &Step, face: &Entity) -> Result<Option<bool>> {
    let surf = st.surface(face)?;
    let loops = face_loops(st, face)?;
    let (Some(surf), Some(loops)) = (surf, loops) else { return Ok(None) };
    if surf.ty == "PLANE" {
        // The polygon of the loop's vertices: their order is trusted, the arcs between them
        // are replaced by chords.
        let mut area = [0.0; 3];
        for segs in &loops {
            if segs.iter().any(|s| s.closed) {
                continue;
            }
            let poly: Vec<V> = segs.iter().map(Segment::start).collect();
            for k in 0..poly.len() {
                area = add(area, cross(poly[k], poly[(k + 1) % poly.len()]));
            }
        }
        let n = st.surface_normal(surf, [0.0; 3])?;
        let Some(n) = n else { return Ok(None) };
        if norm(area) < 1e-9 {
            return Ok(None);
        }
        return Ok(Some(dot(area, n) > 0.0));
    }
    let mut uses: IndexMap<i64, usize> = IndexMap::new();
    for segs in &loops {
        for s in segs {
            *uses.entry(s.edge).or_insert(0) += 1;
        }
    }
    let starts: Vec<V> = loops.iter().flat_map(|segs| segs.iter().map(Segment::start)).collect();
    let Some(centre) = mean(&starts) else { return Ok(None) };
    let mut vote = 0.0;
    for segs in &loops {
        for s in segs {
            if !s.line || uses[&s.edge] > 1 {
                continue; // straight, non-seam edges only
            }
            let (p, q) = (s.points[0], s.points[s.points.len() - 1]);
            let t = unit(sub(q, p));
            let n = st.surface_normal(surf, mul(add(p, q), 0.5))?;
            let (Some(t), Some(n)) = (t, n) else { continue };
            vote += dot(cross(n, t), sub(centre, p));
        }
    }
    if vote.abs() < 1e-9 {
        return Ok(None);
    }
    Ok(Some(vote > 0.0))
}

fn ring_centre(st: &Step, ec_id: i64) -> Result<Option<V>> {
    let c = st.curve(st.e(ec_id)?.a(3)?.r()?)?;
    let Some(c) = c else { return Ok(None) };
    if c.ty != "CIRCLE" && c.ty != "ELLIPSE" {
        return Ok(None);
    }
    Ok(Some(st.placement(c.a(1)?.r()?)?.0))
}

fn revolved(ty: &str) -> bool {
    REVOLVED.contains(&ty)
}

/// For cylinders and cones true is a boss (normal away from the axis), false a hole.
fn sense_from_neighbours(
    st: &Step,
    fid: i64,
    senses: &IndexMap<i64, bool>,
    faces_of_edge: &IndexMap<i64, Vec<i64>>,
) -> Result<Option<bool>> {
    let face = st.e(fid)?;
    let surf = st.surface(face)?;
    let centre = mean(&face_points(st, face)?);
    let (Some(surf), Some(centre)) = (surf, centre) else { return Ok(None) };
    let mut vote: i64 = 0;
    for ec_id in edges_of(st, face)? {
        let Some(ring) = ring_centre(st, ec_id)? else { continue };
        for &other in faces_of_edge.get(&ec_id).map(Vec::as_slice).unwrap_or(&[]) {
            if other == fid || !senses.contains_key(&other) {
                continue;
            }
            let Some(osurf) = st.surface(st.e(other)?)? else { continue };
            let sense = senses[&other];
            if revolved(&surf.ty) && osurf.ty == "PLANE" {
                let axis = st.placement(surf.a(1)?.r()?)?.3;
                let n_plane = st.placement(osurf.a(1)?.r()?)?.3;
                let outward = if sense { n_plane } else { mul(n_plane, -1.0) };
                let along = dot(sub(centre, ring), axis) * dot(axis, outward);
                if along.abs() > 1e-9 {
                    vote += if along > 0.0 { 1 } else { -1 }; // into the material: a hole
                }
            } else if revolved(&surf.ty) && revolved(&osurf.ty) {
                let axis = st.placement(surf.a(1)?.r()?)?.3;
                let o_axis = st.placement(osurf.a(1)?.r()?)?.3;
                if (dot(o_axis, axis).abs() - 1.0).abs() < 1e-6 {
                    vote += if sense { 1 } else { -1 }; // a step or chamfer of the same hole or boss
                }
            } else if surf.ty == "PLANE" && revolved(&osurf.ty) {
                let n_plane = st.placement(surf.a(1)?.r()?)?.3;
                let axis = st.placement(osurf.a(1)?.r()?)?.3;
                let o_centre = mean(&face_points(st, st.e(other)?)?);
                let Some(o_centre) = o_centre else { continue };
                if (dot(axis, n_plane).abs() - 1.0).abs() > 1e-6 {
                    continue;
                }
                let away = sub(ring, o_centre); // the cap of a boss faces away from it
                let outward = if sense { away } else { mul(away, -1.0) }; // a hole floor faces into it
                let d = dot(outward, n_plane);
                if d.abs() > 1e-9 {
                    vote += if d > 0.0 { 1 } else { -1 };
                }
            }
        }
    }
    if vote == 0 {
        return Ok(None);
    }
    Ok(Some(vote > 0))
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Fix {
    pub faces: usize,
    pub flipped: usize,
    pub unknown: usize,
    /// Circular edges whose direction was reversed.
    pub circle_edges: usize,
}

/// For circular edges on cylinders and cones: +1 keeps the edge's same_sense, -1 flips it.
///
/// The face must lie on the left of the edge as travelled, seen from its outward normal: on
/// a cylinder or cone that is along the axis, towards the rest of the face.
pub fn circle_edge_votes(st: &Step, senses: &IndexMap<i64, bool>, faces: &[i64]) -> Result<IndexMap<i64, i64>> {
    let mut votes: IndexMap<i64, i64> = IndexMap::new();
    for &fid in faces {
        let Some(&sense) = senses.get(&fid) else { continue };
        let face = st.e(fid)?;
        let Some(surf) = st.surface(face)? else { continue };
        if !revolved(&surf.ty) {
            continue;
        }
        let axis = st.placement(surf.a(1)?.r()?)?.3;
        let Some(centre) = mean(&face_points(st, face)?) else { continue };
        for b in face.a(1)?.list()? {
            let bound = st.e(b.r()?)?;
            let lp = st.e(bound.a(1)?.r()?)?;
            for oe_ref in lp.a(1)?.list()? {
                let oe = st.e(oe_ref.r()?)?;
                let ec_id = oe.a(3)?.r()?;
                let ec = st.e(ec_id)?;
                let Some(c) = st.curve(ec.a(3)?.r()?)? else { continue };
                if c.ty != "CIRCLE" {
                    continue;
                }
                let (c_o, _x, _y, c_z) = st.placement(c.a(1)?.r()?)?;
                if (dot(c_z, axis).abs() - 1.0).abs() > 1e-6 {
                    continue; // not a ring around this face's axis
                }
                let p = st.vertex(ec.a(1)?.r()?)?;
                let inward = dot(sub(centre, p), axis);
                if inward.abs() < 1e-9 {
                    continue;
                }
                let sigma = (if oe.a(4)?.is(".T.") { 1 } else { -1 }) * (if bound.a(2)?.is(".T.") { 1 } else { -1 });
                let s_curve = if ec.a(4)?.is(".T.") { 1 } else { -1 };
                let t = mul(cross(c_z, sub(p, c_o)), f64::from(sigma * s_curve));
                let Some(mut n) = st.surface_normal(surf, p)? else { continue };
                if !sense {
                    n = mul(n, -1.0);
                }
                let left = dot(cross(n, t), axis) * inward;
                if left.abs() > 1e-12 {
                    *votes.entry(ec_id).or_insert(0) += if left > 0.0 { 1 } else { -1 };
                }
            }
        }
    }
    Ok(votes)
}

pub fn repair_text(text: &str) -> Result<(String, Fix)> {
    let mut st = Step::new(text);
    let faces: Vec<i64> = st.ent.iter().filter(|(_, e)| e.ty == "ADVANCED_FACE").map(|(i, _)| *i).collect();
    let mut faces_of_edge: IndexMap<i64, Vec<i64>> = IndexMap::new();
    for &i in &faces {
        let Ok(edges) = edges_of(&st, &st.ent[&i]) else { continue };
        for ec in edges {
            faces_of_edge.entry(ec).or_default().push(i);
        }
    }
    let mut senses: IndexMap<i64, bool> = IndexMap::new();
    let mut pending: Vec<i64> = Vec::new();
    for &i in &faces {
        match sense_from_loops(&st, &st.ent[&i]).ok().flatten() {
            Some(s) => {
                senses.insert(i, s);
            }
            None => pending.push(i),
        }
    }
    let mut changed = true;
    while !pending.is_empty() && changed {
        // chains of faces settle one after another
        changed = false;
        for i in pending.clone() {
            if let Some(s) = sense_from_neighbours(&st, i, &senses, &faces_of_edge).ok().flatten() {
                senses.insert(i, s);
                pending.retain(|&p| p != i);
                changed = true;
            }
        }
    }
    let mut fix = Fix { faces: faces.len(), unknown: pending.len(), ..Fix::default() };
    for (&i, &sense) in &senses {
        let flag = if sense { ".T." } else { ".F." };
        let line = st.ent[&i].line;
        if st.statements[line].trim_end().ends_with(&format!(",{flag});")) {
            continue;
        }
        if st.set_statement_flag(i, flag)? {
            fix.flipped += 1;
        }
    }
    for (ec_id, vote) in circle_edge_votes(&st, &senses, &faces)? {
        if vote >= 0 {
            continue;
        }
        let flag = if st.e(ec_id)?.a(4)?.is(".T.") { ".F." } else { ".T." };
        if st.set_statement_flag(ec_id, flag)? {
            fix.circle_edges += 1;
        }
    }
    Ok((st.text(), fix))
}

static POINT3: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(CARTESIAN_POINT|DIRECTION)\s*\(\s*('(?:[^']|'')*')\s*,\s*\(\s*([^,()]+)\s*,\s*([^,()]+)\s*,\s*([^,()]+)\s*\)\s*\)")
        .expect("point pattern")
});

/// Turn every 3D point and direction of a STEP file by the rotation m (row-major).
///
/// Two-dimensional points (curves in a surface's parameter space) are left alone. With a
/// proper rotation the orientation of every face and edge is unchanged.
pub fn rotate_text(text: &str, m: &[f64; 9]) -> Result<String> {
    let mut error = None;
    let out = POINT3.replace_all(text, |c: &Captures| {
        let parse = |k: usize| c[k].trim().parse::<f64>();
        match (parse(3), parse(4), parse(5)) {
            (Ok(x), Ok(y), Ok(z)) => {
                let p = [m[0] * x + m[1] * y + m[2] * z, m[3] * x + m[4] * y + m[5] * z, m[6] * x + m[7] * y + m[8] * z];
                format!("{}({},({},{},{}))", &c[1], &c[2], py_repr(p[0]), py_repr(p[1]), py_repr(p[2]))
            }
            _ => {
                error.get_or_insert_with(|| format!("could not convert string to float in {}", &c[0]));
                c[0].to_owned()
            }
        }
    });
    match error {
        Some(e) => Err(anyhow::anyhow!(e)),
        None => Ok(out.into_owned()),
    }
}

pub fn repair_file(path: &Path) -> Result<Fix> {
    let (out, fix) = repair_text(&read_latin1(path)?)?;
    write_latin1(path, &out)?;
    Ok(fix)
}

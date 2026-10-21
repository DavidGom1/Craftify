//! Repair a STEP file's orientation flags using the source CAD's own display mesh.
//! Port of `step_mesh_fix`.
//!
//! cadmpeg 0.6.2 writes SolidWorks parts with wrong orientation flags: every face says it is
//! reversed, most edges run against their curves, and arcs can name the wrong side of their
//! circle. SolidCraft then fills holes with discs, leaves loose cylinders or draws faces that
//! look see-through. The display mesh SolidWorks saved in the part has outward normals and lies
//! on the true surfaces, so it settles each flag directly:
//!
//! * face same_sense: the mesh's outward normals over the face against the surface's normal;
//! * line and B-spline edges: the curve's own direction against the edge's vertex order (exact);
//! * arcs: of the two arcs between an edge's vertices, the one whose middle lies on the mesh;
//! * full circles: travelled so the face is on their left, outer loops counter-clockwise.
//!
//! Faces the mesh cannot settle (no triangles found on them) keep the rule-based answer from
//! [`super::fix`].

use std::f64::consts::PI;

use anyhow::{Result, anyhow};
use indexmap::IndexMap;

use super::fix::{self, circle_edge_votes, edges_of};
use super::{Entity, Step, V, add, at, cross, dot, last, mul, norm, sub, unit};
use crate::mesh::Mesh;

#[derive(Debug, Default, Clone, Copy)]
pub struct MeshFix {
    pub faces: usize,
    pub faces_from_mesh: usize,
    pub faces_from_rules: usize,
    pub faces_unknown: usize,
    pub faces_flipped: usize,
    /// Lines and B-splines re-directed.
    pub curve_edges: usize,
    /// Arcs moved to the other side of their circle.
    pub arcs: usize,
    /// Full circles re-directed.
    pub circles: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FacesMode {
    /// Loop-consistent, from the rules.
    Rules,
    /// The mesh's normals.
    Mesh,
    /// The mesh's answer, turned to agree with the loops when they are all reversed.
    MeshLoops,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub faces_mode: FacesMode,
    pub lines: bool,
    pub arcs: bool,
    pub circles: bool,
}

fn tri_normal(a: V, b: V, c: V) -> Option<V> {
    unit(cross(sub(b, a), sub(c, a)))
}

fn surface_distance(st: &Step, surf: &Entity, p: V) -> Result<Option<f64>> {
    let t = surf.ty.as_str();
    if t == "PLANE" {
        let (o, _x, _y, z) = st.placement(surf.a(1)?.r()?)?;
        return Ok(Some(dot(sub(p, o), z).abs()));
    }
    if matches!(t, "CYLINDRICAL_SURFACE" | "CONICAL_SURFACE" | "SPHERICAL_SURFACE" | "TOROIDAL_SURFACE") {
        let (o, _x, _y, z) = st.placement(surf.a(1)?.r()?)?;
        let d = sub(p, o);
        let axial = dot(d, z);
        let radial = norm(sub(d, mul(z, axial)));
        if t == "CYLINDRICAL_SURFACE" {
            return Ok(Some((radial - surf.a(2)?.f()?).abs()));
        }
        if t == "SPHERICAL_SURFACE" {
            return Ok(Some((norm(d) - surf.a(2)?.f()?).abs()));
        }
        if t == "CONICAL_SURFACE" {
            let mut a = surf.a(3)?.f()?;
            if a.abs() > PI / 2.0 {
                a = a.to_radians();
            }
            let r_at = surf.a(2)?.f()? + axial * a.tan();
            return Ok(Some((radial - r_at).abs() * a.cos()));
        }
        let (major, minor) = (surf.a(2)?.f()?, surf.a(3)?.f()?);
        return Ok(Some(((radial - major).hypot(axial) - minor).abs()));
    }
    Ok(None)
}

/// A B-spline surface's control net, read once per face (Python reads it for every triangle;
/// the answer is the same).
enum Net {
    Grid(Vec<Vec<V>>),
    Unreadable,
}

fn bspline_net(st: &Step, surf: &Entity) -> Net {
    let grid: Result<Vec<Vec<V>>> = (|| {
        surf.a(3)?.list()?.iter().map(|row| row.list()?.iter().map(|r| st.point(r.r()?)).collect()).collect()
    })();
    grid.map_or(Net::Unreadable, Net::Grid)
}

/// Normal of a B-spline surface near p from its control net, and p's distance to the net.
fn bspline_normal(net: &Net, p: V) -> Result<(Option<V>, f64)> {
    let Net::Grid(grid) = net else { return Ok((None, f64::INFINITY)) };
    let (mut best, mut bi, mut bj) = (f64::INFINITY, 0usize, 0usize);
    for (i, row) in grid.iter().enumerate() {
        for (j, q) in row.iter().enumerate() {
            let d = norm(sub(*q, p));
            if d < best {
                (best, bi, bj) = (d, i, j);
            }
        }
    }
    let oob = || anyhow!("list index out of range");
    let n_u = grid.len() as i64;
    let n_v = grid.first().ok_or_else(oob)?.len() as i64;
    let (i0, i1) = ((bi as i64 - 1).max(0), (n_u - 1).min(bi as i64 + 1));
    let (j0, j1) = ((bj as i64 - 1).max(0), (n_v - 1).min(bj as i64 + 1));
    let g = |i: i64, j: i64| -> Result<V> {
        let row = grid.get(i as usize).ok_or_else(oob)?;
        // Python's negative index counts from the end.
        let j = if j < 0 { row.len() as i64 + j } else { j };
        if j < 0 {
            return Err(oob());
        }
        row.get(j as usize).copied().ok_or_else(oob)
    };
    let du = sub(g(i1, bj as i64)?, g(i0, bj as i64)?);
    let dv = sub(g(bi as i64, j1)?, g(bi as i64, j0)?);
    Ok((unit(cross(du, dv)), best))
}

fn face_box(st: &Step, face: &Entity, pad: f64) -> Result<Option<(V, V)>> {
    let mut pts: Vec<V> = Vec::new();
    for b in face.a(1)?.list()? {
        let lp = st.e(st.e(b.r()?)?.a(1)?.r()?)?;
        for oe in lp.a(1)?.list()? {
            let ec = st.e(st.e(oe.r()?)?.a(3)?.r()?)?;
            let v1 = st.vertex(ec.a(1)?.r()?)?;
            let v2 = st.vertex(ec.a(2)?.r()?)?;
            pts.push(v1);
            pts.push(v2);
            let c = st.curve(ec.a(3)?.r()?)?;
            match c {
                Some(c) if c.ty == "CIRCLE" || c.ty == "ELLIPSE" => {
                    let (o, x, y, _z) = st.placement(c.a(1)?.r()?)?;
                    let r = c.a(2)?.f()?;
                    for k in 0..8 {
                        let t = k as f64 * PI / 4.0;
                        pts.push(add(o, add(mul(x, r * t.cos()), mul(y, r * t.sin()))));
                    }
                }
                Some(c) if c.ty.starts_with("B_SPLINE_CURVE") => {
                    let more: Result<Vec<V>> = (|| c.a(2)?.list()?.iter().map(|r| st.point(r.r()?)).collect())();
                    if let Ok(more) = more {
                        pts.extend(more);
                    }
                }
                _ => {}
            }
        }
    }
    if pts.is_empty() {
        return Ok(None);
    }
    let mut lo = pts[0];
    let mut hi = pts[0];
    for p in &pts[1..] {
        for k in 0..3 {
            if p[k] < lo[k] {
                lo[k] = p[k];
            }
            if p[k] > hi[k] {
                hi[k] = p[k];
            }
        }
    }
    Ok(Some(([lo[0] - pad, lo[1] - pad, lo[2] - pad], [hi[0] + pad, hi[1] + pad, hi[2] + pad])))
}

fn in_box(p: V, (lo, hi): (V, V)) -> bool {
    (0..3).all(|k| lo[k] <= p[k] && p[k] <= hi[k])
}

fn face_sense_from_mesh(st: &Step, face: &Entity, centroids: &[(V, V)]) -> Result<Option<bool>> {
    let Some(surf) = st.surface(face)? else { return Ok(None) };
    let Some(bx) = face_box(st, face, 0.5)? else { return Ok(None) };
    let bspline = surf.ty.starts_with("B_SPLINE_SURFACE");
    let net = if bspline { Some(bspline_net(st, surf)) } else { None };
    let mut vote = 0.0;
    for &(c, n) in centroids {
        if !in_box(c, bx) {
            continue;
        }
        let sn = if let Some(net) = &net {
            let (sn, d) = bspline_normal(net, c)?;
            match sn {
                Some(sn) if d <= 2.0 => sn,
                _ => continue,
            }
        } else {
            let Some(d) = surface_distance(st, surf, c)? else { return Ok(None) };
            let r = if surf.ty != "PLANE" { surf.a(2)?.f()? } else { 0.0 };
            if d > 0.02_f64.max(0.03 * r) {
                continue;
            }
            let Some(sn) = st.surface_normal(surf, c)? else { continue };
            sn
        };
        let d = dot(sn, n);
        if d.abs() < 0.5 {
            continue; // the triangle is not along this surface there
        }
        vote += if d > 0.0 { 1.0 } else { -1.0 };
    }
    if vote == 0.0 {
        return Ok(None);
    }
    Ok(Some(vote > 0.0))
}

/// Middle of the counter-clockwise arc (about the circle's axis) from a to b.
fn arc_mid(st: &Step, circle: &Entity, a: V, b: V) -> Result<V> {
    let (o, x, y, _z) = st.placement(circle.a(1)?.r()?)?;
    let rx = circle.a(2)?.f()?;
    let ry = if circle.ty == "CIRCLE" { rx } else { circle.a(3)?.f()? };
    let ang = |p: V| {
        let d = sub(p, o);
        dot(d, y).atan2(dot(d, x))
    };
    let t0 = ang(a);
    let span = super::py_mod(ang(b) - t0, 2.0 * PI);
    let t = t0 + span / 2.0;
    Ok(add(o, add(mul(x, rx * t.cos()), mul(y, ry * t.sin()))))
}

fn loop_extent(st: &Step, bound: &Entity) -> Result<f64> {
    let mut pts = Vec::new();
    let lp = st.e(bound.a(1)?.r()?)?;
    let edges = lp.a(1)?;
    for oe in edges.list()? {
        let ec = st.e(st.e(oe.r()?)?.a(3)?.r()?)?;
        pts.push(st.vertex(ec.a(1)?.r()?)?);
        if let Some(c) = st.curve(ec.a(3)?.r()?)?
            && (c.ty == "CIRCLE" || c.ty == "ELLIPSE") {
                let d = 2.0 * c.a(2)?.f()?;
                return Ok(if edges.py_len() == 1 { d } else { d.max(0.0) });
            }
    }
    if pts.len() < 2 {
        return Ok(0.0);
    }
    let mut best = f64::NEG_INFINITY;
    for p in &pts {
        for q in &pts {
            let d = norm(sub(*p, *q));
            if d > best {
                best = d;
            }
        }
    }
    Ok(best)
}

/// Does the edge run along its curve's own direction? Exact for lines and B-splines.
fn geometric_edge_sense(st: &Step, ec: &Entity) -> Result<Option<bool>> {
    let Some(c) = st.curve(ec.a(3)?.r()?)? else { return Ok(None) };
    let v1 = st.vertex(ec.a(1)?.r()?)?;
    let v2 = st.vertex(ec.a(2)?.r()?)?;
    if c.ty == "LINE" {
        let vec = st.get(c.a(2)?.r()?);
        let Some(vec) = vec.filter(|v| v.ty == "VECTOR") else { return Ok(None) };
        let d = dot(st.direction(vec.a(1)?.r()?)?, sub(v2, v1));
        return Ok(if d.abs() < 1e-12 { None } else { Some(d > 0.0) });
    }
    if c.ty.starts_with("B_SPLINE_CURVE") {
        if ec.a(1)? == ec.a(2)? {
            return Ok(None);
        }
        let ends: Result<(V, V)> = (|| {
            let pts = c.a(2)?.list()?;
            Ok((st.point(at(pts, 0)?.r()?)?, st.point(last(pts)?.r()?)?))
        })();
        let Ok((first, lastp)) = ends else { return Ok(None) };
        let same = norm(sub(first, v1)) + norm(sub(lastp, v2));
        let rev = norm(sub(first, v2)) + norm(sub(lastp, v1));
        if (same - rev).abs() < 1e-9 {
            return Ok(None);
        }
        return Ok(Some(same < rev));
    }
    Ok(None)
}

pub fn repair_text(text: &str, mesh: &Mesh, o: Options) -> Result<(String, MeshFix)> {
    let mut fix = MeshFix::default();
    let (ruled_text, _) = fix::repair_text(text)?; // rule-based face flags and circle votes
    let mut st = Step::new(if o.faces_mode == FacesMode::Rules { &ruled_text } else { text });

    // 1. lines and B-splines: exact
    if o.lines {
        let ids: Vec<i64> = st.ent.keys().copied().collect();
        for id in ids {
            if st.ent[&id].ty != "EDGE_CURVE" {
                continue;
            }
            if let Some(g) = geometric_edge_sense(&st, &st.ent[&id])?
                && st.set_last_flag(id, g)? {
                    fix.curve_edges += 1;
                }
        }
    }

    // 2. faces
    let faces: Vec<i64> = st.ent.iter().filter(|(_, e)| e.ty == "ADVANCED_FACE").map(|(i, _)| *i).collect();
    fix.faces = faces.len();
    let mut senses: IndexMap<i64, bool> = IndexMap::new();
    for &i in &faces {
        senses.insert(i, st.e(i)?.a(3)?.is(".T."));
    }
    if o.faces_mode != FacesMode::Rules {
        let rst = Step::new(&ruled_text);
        let ruled = |i: i64| -> Result<bool> { Ok(rst.e(i)?.a(3)?.is(".T.")) };
        let mut centroids = Vec::new();
        for [a, b, c] in &mesh.triangles {
            if let Some(n) = tri_normal(*a, *b, *c) {
                let mid = |k: usize| crate::util::py_sum([a[k], b[k], c[k]]) / 3.0;
                centroids.push(([mid(0), mid(1), mid(2)], n));
            }
        }
        let mut from_mesh: IndexMap<i64, bool> = IndexMap::new();
        for &i in &faces {
            if let Some(s) = face_sense_from_mesh(&st, &st.ent[&i], &centroids).ok().flatten() {
                from_mesh.insert(i, s);
            }
        }
        let mut flip_all = false;
        if o.faces_mode == FacesMode::MeshLoops && !from_mesh.is_empty() {
            let mut agree = 0usize;
            for (&i, &s) in &from_mesh {
                if s == ruled(i)? {
                    agree += 1;
                }
            }
            flip_all = (agree as f64) < from_mesh.len() as f64 / 2.0; // the loops run reversed throughout
        }
        for &i in &faces {
            if let Some(&s) = from_mesh.get(&i) {
                senses.insert(i, if flip_all { !s } else { s });
                fix.faces_from_mesh += 1;
            } else {
                senses.insert(i, ruled(i)?);
                fix.faces_from_rules += 1;
            }
        }
        for (&i, &s) in &senses {
            if st.set_last_flag(i, s)? {
                fix.faces_flipped += 1;
            }
        }
    }
    if !o.arcs && !o.circles {
        return Ok((st.text(), fix));
    }

    // 3. arcs: the side whose middle lies on the mesh
    if o.arcs {
        let ids: Vec<i64> = st.ent.keys().copied().collect();
        for id in ids {
            let e = &st.ent[&id];
            if e.ty != "EDGE_CURVE" || e.a(1)? == e.a(2)? {
                continue;
            }
            let Some(c) = st.curve(e.a(3)?.r()?)? else { continue };
            if c.ty != "CIRCLE" && c.ty != "ELLIPSE" {
                continue;
            }
            let v1 = st.vertex(e.a(1)?.r()?)?;
            let v2 = st.vertex(e.a(2)?.r()?)?;
            let r = c.a(2)?.f()?;
            let mid_t = arc_mid(&st, c, v1, v2)?; // same_sense .T.: counter-clockwise from v1 to v2
            let mid_f = arc_mid(&st, c, v2, v1)?; // .F.: the other side
            let dt = mesh.signed_distance(mid_t, r + 1.0).map_or(f64::INFINITY, f64::abs);
            let df = mesh.signed_distance(mid_f, r + 1.0).map_or(f64::INFINITY, f64::abs);
            if dt.min(df) == f64::INFINITY || (dt - df).abs() < 1e-6 {
                continue;
            }
            if st.set_last_flag(id, dt < df)? {
                fix.arcs += 1;
            }
        }
    }

    // 4. full circles: the face on their left
    if !o.circles {
        return Ok((st.text(), fix));
    }
    for &i in &faces {
        edges_of(&st, &st.ent[&i])?; // Python builds faces_of_edge here, failing alike
    }
    let mut votes = circle_edge_votes(&st, &senses, &faces)?; // cylinders and cones
    for &i in &faces {
        let face = &st.ent[&i];
        let Some(surf) = st.surface(face)? else { continue };
        if surf.ty != "PLANE" || !senses.contains_key(&i) {
            continue;
        }
        let mut n_out = st.placement(surf.a(1)?.r()?)?.3;
        if !senses[&i] {
            n_out = mul(n_out, -1.0);
        }
        let mut bounds = Vec::new();
        for b in face.a(1)?.list()? {
            bounds.push(st.e(b.r()?)?);
        }
        let mut extents = Vec::new();
        for b in &bounds {
            extents.push(loop_extent(&st, b)?);
        }
        // max(range(n), key=...) keeps the first of equal extents
        let mut biggest: Option<usize> = None;
        for k in 0..extents.len() {
            if biggest.is_none_or(|b| extents[k] > extents[b]) {
                biggest = Some(k);
            }
        }
        for (k, bound) in bounds.iter().enumerate() {
            let lp = st.e(bound.a(1)?.r()?)?;
            if lp.a(1)?.py_len() != 1 {
                continue;
            }
            let oe = st.e(at(lp.a(1)?.list()?, 0)?.r()?)?;
            let ec_id = oe.a(3)?.r()?;
            let ec = st.e(ec_id)?;
            let Some(c) = st.curve(ec.a(3)?.r()?)? else { continue };
            if c.ty != "CIRCLE" || ec.a(1)? != ec.a(2)? {
                continue;
            }
            let cz = st.placement(c.a(1)?.r()?)?.3;
            let sigma = (if oe.a(4)?.is(".T.") { 1.0 } else { -1.0 }) * (if bound.a(2)?.is(".T.") { 1.0 } else { -1.0 });
            let s_curve = if ec.a(4)?.is(".T.") { 1.0 } else { -1.0 };
            let ccw = dot(cz, n_out) * sigma * s_curve > 0.0;
            let want_ccw = Some(k) == biggest;
            *votes.entry(ec_id).or_insert(0) += if ccw == want_ccw { 1 } else { -1 };
        }
    }
    for (ec_id, vote) in votes {
        let e = st.e(ec_id)?;
        if vote < 0 && e.a(1)? == e.a(2)? {
            // arcs were settled from the mesh already
            let value = !e.a(4)?.is(".T.");
            if st.set_last_flag(ec_id, value)? {
                fix.circles += 1;
            }
        }
    }
    Ok((st.text(), fix))
}

/// The ways to repair cadmpeg's orientation flags, best first over the test parts; each part
/// keeps the one whose result matches SolidWorks' own display mesh best.
pub const REPAIRS: [(&str, Options); 6] = [
    ("malla+bordes", Options { faces_mode: FacesMode::MeshLoops, lines: true, arcs: true, circles: true }),
    ("reglas", Options { faces_mode: FacesMode::Rules, lines: false, arcs: false, circles: false }),
    ("reglas+rectas+arcos", Options { faces_mode: FacesMode::Rules, lines: true, arcs: true, circles: false }),
    ("malla+arcos", Options { faces_mode: FacesMode::MeshLoops, lines: false, arcs: true, circles: true }),
    ("reglas+arcos", Options { faces_mode: FacesMode::Rules, lines: false, arcs: true, circles: false }),
    ("reglas+arcos+círculos", Options { faces_mode: FacesMode::Rules, lines: false, arcs: true, circles: true }),
];

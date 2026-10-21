//! Tidy a STEP file written by cadmpeg the way Fusion's own STEP export would write it.
//! Port of `step_clean.py`.
//!
//! SolidCraft reads some of what cadmpeg writes wrong (faces trimmed to half their area, gaps
//! between faces), though it is valid STEP. Fusion's own export differs in three ways, which
//! this module copies:
//!
//! * Seams. A face that goes all the way round a cylinder or cone comes from cadmpeg the way the
//!   source CAD keeps it: two loops, one at each end (or one loop and the cone's apex), and no
//!   seam. Fusion cuts it open along a ruling line, so the face has a single loop that runs along
//!   the seam once in each direction. The seam runs through a vertex of one loop; where the other
//!   loop has no vertex at that angle, the edge that crosses it is split there (both faces that
//!   use the edge get the two halves).
//! * Trimmed curves. cadmpeg gives an edge the whole cached B-spline of its curve, which can run
//!   well past the edge's ends; Fusion writes only the stretch between the edge's vertices.
//! * Tolerance. The vertices of the cached curves sit a few micrometres off them; the declared
//!   model tolerance must allow for that.
//!
//! Each step is tried per face or edge and, as in Python, a step that raises a KeyError,
//! ValueError, IndexError, ZeroDivisionError or StopIteration is skipped (what it changed before
//! failing stays); any other failure ends the clean-up.

use std::collections::HashMap;
use std::f64::consts::PI;
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;

use super::edit::{Edit, fmt, r, s};
use super::{Arg, Entity, PyKind, V, add, div, dot, kind_of, mul, norm, py_err, py_mod, sub, unit};
use crate::pyset::PySet;
use crate::util::py_repr;

const ROUND: [&str; 2] = ["CYLINDRICAL_SURFACE", "CONICAL_SURFACE"];
const TORUS: &str = "TOROIDAL_SURFACE";
/// mm: a split this close to a vertex uses the vertex.
const SAME_POINT: f64 = 1e-6;

fn index_err() -> anyhow::Error {
    py_err(PyKind::Index, "list index out of range")
}

/// `v[i]` with Python's negative indices.
fn pyget<T: Copy>(v: &[T], i: i64) -> Result<T> {
    let j = if i < 0 { i + v.len() as i64 } else { i };
    if j < 0 { Err(index_err()) } else { v.get(j as usize).copied().ok_or_else(index_err) }
}

/// `v[a:b]` with Python's slice rules.
fn pyslice<T: Clone>(v: &[T], a: i64, b: i64) -> Vec<T> {
    let n = v.len() as i64;
    let fix = |i: i64| if i < 0 { (i + n).max(0) } else { i.min(n) };
    let (a, b) = (fix(a), fix(b));
    if a >= b { Vec::new() } else { v[a as usize..b as usize].to_vec() }
}

/// Python's `max(a, b)` and `min(a, b)` (the first argument unless the second is beyond it).
fn pymax(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}
fn pymin(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

// --------------------------------------------------------------------------- curves

fn wrap(a: f64) -> f64 {
    py_mod(a + PI, 2.0 * PI) - PI
}

enum Kind {
    None,
    Line { o: V, d: V },
    Conic { o: V, x: V, y: V, a: f64, b: f64 },
    Spline { p: i64, ctrl: Vec<V>, w: Vec<f64>, u: Vec<f64> },
}

/// A curve that can be evaluated at a parameter: lines, circles, ellipses and B-splines.
struct Curve {
    kind: Kind,
    table: Option<(Vec<f64>, Vec<V>)>,
}

impl Curve {
    fn new(st: &Edit, cid: i64) -> Result<Curve> {
        let mut cid = cid;
        let mut e = st.st.get(cid);
        while let Some(x) = e {
            if x.ty != "SURFACE_CURVE" && x.ty != "SEAM_CURVE" {
                break;
            }
            cid = x.a(1)?.r()?;
            e = st.st.get(cid);
        }
        let kind = match e {
            Some(e) if e.ty == "LINE" => {
                let o = st.st.point(e.a(1)?.r()?)?;
                let vec = st.st.e(e.a(2)?.r()?)?;
                let dir = st.st.direction(vec.a(1)?.r()?)?;
                Kind::Line { o, d: mul(dir, vec.a(2)?.f()?) }
            }
            Some(e) if e.ty == "CIRCLE" || e.ty == "ELLIPSE" => {
                let (o, x, y, _z) = st.st.placement(e.a(1)?.r()?)?;
                let a = e.a(2)?.f()?;
                let b = if e.ty == "CIRCLE" { a } else { e.a(3)?.f()? };
                Kind::Conic { o, x, y, a, b }
            }
            _ => {
                let parts = st.complex.get(&cid).filter(|p| !p.is_empty());
                let (deg, pts, mults, knots, weights): (&Arg, &Arg, &Arg, &Arg, Option<&Arg>);
                if let Some(e) = e.filter(|e| e.ty == "B_SPLINE_CURVE_WITH_KNOTS") {
                    (deg, pts, mults, knots, weights) = (e.a(1)?, e.a(2)?, e.a(6)?, e.a(7)?, None);
                } else if let Some(p) = parts.filter(|p| p.contains_key("B_SPLINE_CURVE") && p.contains_key("B_SPLINE_CURVE_WITH_KNOTS")) {
                    let bs = &p["B_SPLINE_CURVE"];
                    let bk = &p["B_SPLINE_CURVE_WITH_KNOTS"];
                    (deg, pts) = (bs.first().ok_or_else(index_err)?, bs.get(1).ok_or_else(index_err)?);
                    (mults, knots) = (bk.first().ok_or_else(index_err)?, bk.get(1).ok_or_else(index_err)?);
                    weights = match p.get("RATIONAL_B_SPLINE_CURVE") {
                        Some(w) => Some(w.first().ok_or_else(index_err)?),
                        None => None,
                    };
                } else {
                    return Ok(Curve { kind: Kind::None, table: None });
                }
                let p = deg.i()?;
                if p < 0 {
                    return Err(py_err(PyKind::Value, "negative degree"));
                }
                let ctrl: Vec<V> = pts.list()?.iter().map(|r| st.st.point(r.r()?)).collect::<Result<_>>()?;
                let w = match weights {
                    Some(Arg::L(l)) if !l.is_empty() => l.iter().map(Arg::f).collect::<Result<_>>()?,
                    Some(Arg::S(t)) if !t.is_empty() => return Err(py_err(PyKind::Value, "weights")),
                    _ => vec![1.0; ctrl.len()],
                };
                let mut u = Vec::new();
                for (k, m) in knots.list()?.iter().zip(mults.list()?) {
                    for _ in 0..m.i()?.max(0) {
                        u.push(k.f()?);
                    }
                }
                Kind::Spline { p, ctrl, w, u }
            }
        };
        Ok(Curve { kind, table: None })
    }

    fn is_none(&self) -> bool {
        matches!(self.kind, Kind::None)
    }

    fn at(&self, t: f64) -> Result<V> {
        match &self.kind {
            Kind::Line { o, d } => Ok(add(*o, mul(*d, t))),
            Kind::Conic { o, x, y, a, b } => Ok(add(*o, add(mul(*x, a * t.cos()), mul(*y, b * t.sin())))),
            Kind::None => Err(py_err(PyKind::Type, "no curve")),
            Kind::Spline { p, ctrl, w, u } => {
                let (p, n) = (*p, ctrl.len() as i64 - 1);
                let t = pymin(pymax(t, pyget(u, p)?), pyget(u, n + 1)?);
                let mut k = p;
                while k < n && t >= pyget(u, k + 1)? {
                    k += 1;
                }
                let mut d: Vec<[f64; 4]> = Vec::with_capacity((p + 1) as usize);
                for j in k - p..=k {
                    let (c, wj) = (pyget(ctrl, j)?, pyget(w, j)?);
                    let m = mul(c, wj);
                    d.push([m[0], m[1], m[2], wj]);
                }
                for r in 1..=p {
                    for j in (r..=p).rev() {
                        let (lo, hi) = (pyget(u, j + k - p)?, pyget(u, j + 1 + k - r)?);
                        let a = if hi == lo { 0.0 } else { (t - lo) / (hi - lo) };
                        let (dj1, dj) = (d[(j - 1) as usize], d[j as usize]);
                        d[j as usize] = [0, 1, 2, 3].map(|m| (1.0 - a) * dj1[m] + a * dj[m]);
                    }
                }
                let x = d[p as usize];
                Ok([div(x[0], x[3])?, div(x[1], x[3])?, div(x[2], x[3])?])
            }
        }
    }

    fn domain(&self) -> Result<(f64, f64)> {
        Ok(match &self.kind {
            Kind::Spline { p, ctrl, u, .. } => (pyget(u, *p)?, pyget(u, ctrl.len() as i64)?),
            Kind::Conic { .. } => (0.0, 2.0 * PI),
            _ => (0.0, 1.0),
        })
    }

    /// The parameter of the curve's point nearest q.
    fn param(&mut self, q: V) -> Result<f64> {
        match &self.kind {
            Kind::Line { o, d } => {
                let dd = dot(*d, *d);
                return Ok(if dd != 0.0 { dot(sub(q, *o), *d) / dd } else { 0.0 });
            }
            Kind::Conic { o, x, y, a, b } => {
                let d = sub(q, *o);
                let sy = div(dot(d, *y), *b)?;
                let sx = div(dot(d, *x), *a)?;
                return Ok(sy.atan2(sx));
            }
            _ => {}
        }
        if self.table.is_none() {
            let (lo, hi) = self.domain()?;
            let n = match &self.kind {
                Kind::Spline { ctrl, .. } => (16 * ctrl.len()).max(64),
                _ => 64,
            };
            let ts: Vec<f64> = (0..=n).map(|i| lo + (hi - lo) * i as f64 / n as f64).collect();
            let pts = ts.iter().map(|t| self.at(*t)).collect::<Result<Vec<V>>>()?;
            self.table = Some((ts, pts));
        }
        let (ts, pts) = self.table.as_ref().ok_or_else(index_err)?;
        let mut i = 0;
        let mut best = f64::INFINITY;
        for (j, p) in pts.iter().enumerate() {
            let d = norm(sub(*p, q));
            if j == 0 || d < best {
                (best, i) = (d, j);
            }
        }
        let (mut a, mut b) = (ts[i.saturating_sub(1)], ts[(i + 1).min(ts.len() - 1)]);
        for _ in 0..60 {
            // golden section
            let (m1, m2) = (b - (b - a) * 0.618, a + (b - a) * 0.618);
            if norm(sub(self.at(m1)?, q)) < norm(sub(self.at(m2)?, q)) {
                b = m2;
            } else {
                a = m1;
            }
        }
        Ok((a + b) / 2.0)
    }
}

type Curves = HashMap<i64, Curve>;

fn curve<'c>(st: &Edit, cid: i64, curves: &'c mut Curves) -> Result<&'c mut Curve> {
    if !curves.contains_key(&cid) {
        let c = Curve::new(st, cid)?;
        curves.insert(cid, c);
    }
    curves.get_mut(&cid).ok_or_else(index_err)
}

/// One oriented edge of a loop, as travelled.
#[derive(Clone)]
struct Travel {
    edge: i64,
    /// Travelled from its start vertex to its end vertex.
    forward: bool,
    /// Vertex ids, in travel order.
    start: i64,
    params: Vec<f64>,
    points: Vec<V>,
}

fn travel(st: &Edit, ec_id: i64, forward: bool, curves: &mut Curves) -> Result<Option<Travel>> {
    let ec = st.st.e(ec_id)?; // EDGE_CURVE('', v1, v2, curve, same_sense)
    let (v1, v2) = (ec.a(1)?.r()?, ec.a(2)?.r()?);
    let cid = ec.a(3)?.r()?;
    let c = curve(st, cid, curves)?;
    if c.is_none() {
        return Ok(None);
    }
    let (p1, p2) = (st.st.vertex(v1)?, st.st.vertex(v2)?);
    let (mut t1, mut t2) = (c.param(p1)?, c.param(p2)?);
    let along = ec.a(4)?.is(".T."); // v1 -> v2 runs the curve's own way
    if matches!(c.kind, Kind::Conic { .. }) {
        let mut span = if along { py_mod(t2 - t1, 2.0 * PI) } else { -py_mod(t1 - t2, 2.0 * PI) };
        if v1 == v2 || span.abs() < 1e-12 {
            span = if along { 2.0 * PI } else { -2.0 * PI };
        }
        t2 = t1 + span;
    } else if v1 == v2 {
        let (lo, hi) = c.domain()?;
        (t1, t2) = if along { (lo, hi) } else { (hi, lo) };
    }
    let n = if matches!(c.kind, Kind::Line { .. }) { 2 } else { 24 };
    let ts: Vec<f64> = (0..=n).map(|i| t1 + (t2 - t1) * i as f64 / n as f64).collect();
    let mut pts = ts.iter().map(|t| c.at(*t)).collect::<Result<Vec<V>>>()?;
    pts[0] = p1;
    pts[n] = p2;
    Ok(Some(if forward {
        Travel { edge: ec_id, forward: true, start: v1, params: ts, points: pts }
    } else {
        Travel {
            edge: ec_id,
            forward: false,
            start: v2,
            params: ts.into_iter().rev().collect(),
            points: pts.into_iter().rev().collect(),
        }
    }))
}

#[derive(Clone)]
struct Loop {
    bound: i64,
    /// Empty for a vertex loop.
    steps: Vec<Travel>,
    /// The vertex of a vertex loop.
    pole: Option<i64>,
}

fn loops(st: &Edit, face: &Entity, curves: &mut Curves) -> Result<Option<Vec<Loop>>> {
    let mut out = Vec::new();
    for b in face.a(1)?.list()? {
        let bound = st.st.e(b.r()?)?;
        let lp = st.st.e(bound.a(1)?.r()?)?;
        let same = bound.a(2)?.is(".T.");
        if lp.ty == "VERTEX_LOOP" {
            out.push(Loop { bound: b.r()?, steps: Vec::new(), pole: Some(lp.a(1)?.r()?) });
            continue;
        }
        let mut steps = Vec::new();
        for oe_ref in lp.a(1)?.list()? {
            let oe = st.st.e(oe_ref.r()?)?;
            let Some(s) = travel(st, oe.a(3)?.r()?, oe.a(4)?.is(".T.") == same, curves)? else { return Ok(None) };
            steps.push(s);
        }
        if !same {
            steps.reverse();
        }
        out.push(Loop { bound: b.r()?, steps, pole: None });
    }
    Ok(Some(out))
}

// --------------------------------------------------------------------------- seams

struct Axis {
    o: V,
    x: V,
    y: V,
    z: V,
}

impl Axis {
    fn new(st: &Edit, surf: &Entity) -> Result<Axis> {
        let (o, x, y, z) = st.st.placement(surf.a(1)?.r()?)?;
        Ok(Axis { o, x, y, z })
    }
    fn angle(&self, p: V) -> f64 {
        let d = sub(p, self.o);
        dot(d, self.y).atan2(dot(d, self.x))
    }
    fn radius(&self, p: V) -> f64 {
        let d = sub(p, self.o);
        norm(sub(d, mul(self.z, dot(d, self.z))))
    }
    fn winding(&self, lp: &Loop) -> f64 {
        let (mut total, mut last) = (0.0, None);
        for s in &lp.steps {
            for p in &s.points {
                let a = self.angle(*p);
                if let Some(l) = last {
                    total += wrap(a - l);
                }
                last = Some(a);
            }
        }
        total / (2.0 * PI)
    }
}

/// Turns a loop makes round a torus's tube.
fn tube_winding(surf: &Entity, ax: &Axis, lp: &Loop) -> Result<f64> {
    let major = surf.a(2)?.f()?;
    let (mut total, mut last) = (0.0, None);
    for s in &lp.steps {
        for p in &s.points {
            let d = sub(*p, ax.o);
            let v = dot(d, ax.z).atan2(dot(d, ax.x).hypot(dot(d, ax.y)) - major);
            if let Some(l) = last {
                total += wrap(v - l);
            }
            last = Some(v);
        }
    }
    Ok(total / (2.0 * PI))
}

enum Hit {
    /// At a vertex of the loop.
    Vertex(i64),
    /// Inside the k-th edge, at a point of its curve.
    Cut(usize, V),
}

/// Where the loop passes the angle theta: at a vertex, or inside an edge.
fn crossing(st: &Edit, ax: &Axis, lp: &Loop, theta: f64, curves: &Curves) -> Result<Option<Hit>> {
    for s in &lp.steps {
        let v = st.st.vertex(s.start)?;
        if ax.radius(v) * wrap(ax.angle(v) - theta).abs() < SAME_POINT * 100.0 {
            return Ok(Some(Hit::Vertex(s.start)));
        }
    }
    for (k, s) in lp.steps.iter().enumerate() {
        let cid = st.st.e(s.edge)?.a(3)?.r()?;
        let c = curves.get(&cid);
        let c = || c.ok_or_else(|| py_err(PyKind::Type, "'NoneType' object has no attribute 'at'"));
        for i in 0..s.points.len().saturating_sub(1) {
            let a = wrap(ax.angle(s.points[i]) - theta);
            let b = wrap(ax.angle(s.points[i + 1]) - theta);
            if b.abs() < 1e-12 && i + 1 < s.points.len() - 1 {
                let t = s.params[i + 1];
                return Ok(Some(Hit::Cut(k, c()?.at(t)?)));
            }
            if a == 0.0 || (a < 0.0) == (b < 0.0) || (a - b).abs() > PI {
                continue;
            }
            let (mut lo, mut hi) = (s.params[i], s.params[i + 1]);
            for _ in 0..80 {
                let mid = (lo + hi) / 2.0;
                let m = wrap(ax.angle(c()?.at(mid)?) - theta);
                if (m < 0.0) == (a < 0.0) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let t = (lo + hi) / 2.0;
            return Ok(Some(Hit::Cut(k, c()?.at(t)?)));
        }
    }
    Ok(None)
}

/// Split an edge at p (a point of its curve); every loop that uses it gets both halves.
fn split(st: &mut Edit, ec_id: i64, p: V) -> Result<i64> {
    let ec = st.st.e(ec_id)?.clone();
    let pt = st.point_entity(p);
    let vp = st.add("VERTEX_POINT", vec![s("''"), r(pt)]);
    let first_args = vec![s("''"), ec.a(1)?.clone(), r(vp), ec.a(3)?.clone(), ec.a(4)?.clone()];
    let first = st.add("EDGE_CURVE", first_args);
    let second_args = vec![s("''"), r(vp), ec.a(2)?.clone(), ec.a(3)?.clone(), ec.a(4)?.clone()];
    let second = st.add("EDGE_CURVE", second_args);
    for oe_id in st.users_of(ec_id) {
        let oe = st.st.e(oe_id)?.clone();
        if oe.ty != "ORIENTED_EDGE" {
            continue;
        }
        let oe_edge = |st: &mut Edit, edge: i64, flag: &str| st.add("ORIENTED_EDGE", vec![s("''"), s("*"), s("*"), r(edge), s(flag)]);
        let pair = if oe.a(4)?.is(".T.") {
            [oe_edge(st, first, ".T."), oe_edge(st, second, ".T.")]
        } else {
            [oe_edge(st, second, ".F."), oe_edge(st, first, ".F.")]
        };
        for loop_id in st.users_of(oe_id) {
            let lp = st.st.e(loop_id)?.clone();
            let me = format!("#{oe_id}");
            let mut items = Vec::new();
            for a in lp.a(1)?.list()? {
                if a.is(&me) {
                    items.extend(pair.iter().map(|i| r(*i)));
                } else {
                    items.push(a.clone());
                }
            }
            st.set(loop_id, vec![lp.a(0)?.clone(), Arg::L(items)])?;
        }
        st.drop(oe_id)?;
    }
    st.drop(ec_id)?;
    Ok(vp)
}

fn seam(st: &mut Edit, va: i64, vb: i64) -> Result<i64> {
    let (pa, pb) = (st.st.vertex(va)?, st.st.vertex(vb)?);
    let d = sub(pb, pa);
    let u = unit(d).unwrap_or([0.0, 0.0, 1.0]);
    let direction = st.add("DIRECTION", vec![s("''"), Arg::L(vec![s(py_repr(u[0])), s(py_repr(u[1])), s(py_repr(u[2]))])]);
    let vector = st.add("VECTOR", vec![s("''"), r(direction), s(py_repr(norm(d)))]);
    let pt = st.point_entity(pa);
    let line = st.add("LINE", vec![s("''"), r(pt), r(vector)]);
    Ok(st.add("EDGE_CURVE", vec![s("''"), r(va), r(vb), r(line), s(".T.")]))
}

/// The seam of a torus face that goes round the torus axis: an arc of the tube's circle half a
/// turn from the reference direction, from va to vb, the way the tube angle rises or falls.
fn meridian(st: &mut Edit, surf: &Entity, ax: &Axis, va: i64, vb: i64, rising: bool) -> Result<i64> {
    let major = surf.a(2)?.f()?;
    let out = mul(ax.x, -1.0); // radial direction half a turn round
    let centre = add(ax.o, mul(out, major));
    let dir = |st: &mut Edit, v: V| st.add("DIRECTION", vec![s("''"), Arg::L(v.iter().map(|c| s(py_repr(*c))).collect())]);
    // axis y: the circle's own parameter is then the tube angle (y x -x = z)
    let axis = dir(st, ax.y);
    let reference = dir(st, out);
    let pt = st.point_entity(centre);
    let place = st.add("AXIS2_PLACEMENT_3D", vec![s("''"), r(pt), r(axis), r(reference)]);
    let circle = st.add("CIRCLE", vec![s("''"), r(place), surf.a(3)?.clone()]);
    Ok(st.add("EDGE_CURVE", vec![s("''"), r(va), r(vb), r(circle), s(if rising { ".T." } else { ".F." })]))
}

fn rotated(steps: &[Travel], v: i64) -> Result<Vec<Travel>> {
    let k = steps.iter().position(|s| s.start == v).ok_or_else(|| py_err(PyKind::StopIteration, ""))?;
    Ok(steps[k..].iter().chain(&steps[..k]).cloned().collect())
}

fn cut_open(st: &mut Edit, fid: i64, curves: &mut Curves) -> Result<bool> {
    let face = st.st.e(fid)?.clone();
    let surf = st.st.get(face.a(2)?.r()?).cloned();
    let Some(surf) = surf.filter(|s| ROUND.contains(&s.ty.as_str()) || s.ty == TORUS) else { return Ok(false) };
    let Some(all) = loops(st, &face, curves)? else { return Ok(false) };
    let ax = Axis::new(st, &surf)?;
    let round: Vec<&Loop> = all.iter().filter(|lp| !lp.steps.is_empty() && ax.winding(lp).abs() > 0.5).collect();
    if surf.ty == TORUS {
        // only rings round the torus axis (fillets), not loops round the tube
        if round.len() != 2 {
            return Ok(false);
        }
        for lp in &round {
            if tube_winding(&surf, &ax, lp)?.abs() > 0.5 {
                return Ok(false);
            }
        }
    }
    let poles: Vec<&Loop> = all.iter().filter(|lp| lp.pole.is_some()).collect();
    let (a, b) = if round.len() == 2 {
        (round[0].clone(), round[1].clone())
    } else if round.len() == 1 && poles.len() == 1 && surf.ty == "CONICAL_SURFACE" {
        (round[0].clone(), poles[0].clone())
    } else {
        return Ok(false);
    };

    // Where Fusion puts it: half a turn from the surface's reference direction (SolidCraft
    // trims these faces right only with the seam there).
    let mut ends = Vec::new();
    for lp in [&a, &b] {
        if let Some(p) = lp.pole {
            ends.push(p);
            continue;
        }
        let Some(hit) = crossing(st, &ax, lp, PI, curves)? else { return Ok(false) };
        let v = match hit {
            Hit::Vertex(v) => v,
            Hit::Cut(k, p) => split(st, lp.steps[k].edge, p)?,
        };
        ends.push(v);
    }
    // again, with any split edges
    let face_now = st.st.e(fid)?.clone();
    let again = loops(st, &face_now, curves)?.unwrap_or_default();
    let by_bound: HashMap<i64, Loop> = again.into_iter().map(|lp| (lp.bound, lp)).collect();
    let (Some(a), Some(b)) = (by_bound.get(&a.bound).cloned(), by_bound.get(&b.bound).cloned()) else { return Ok(false) };
    let face = st.st.e(fid)?.clone();
    let (va, vb) = (ends[0], ends[1]);

    let seam = if surf.ty == TORUS {
        // The face lies left of its loops seen from its normal: on the torus that is the side of
        // rising tube angle for the loop that goes round the axis the positive way.
        let a_positive = ax.winding(&a) > 0.0;
        meridian(st, &surf, &ax, va, vb, a_positive == face.a(3)?.is(".T."))?
    } else {
        seam(st, va, vb)?
    };
    let mut items = Vec::new();
    let oe = |st: &mut Edit, edge: i64, forward: bool| {
        st.add("ORIENTED_EDGE", vec![s("''"), s("*"), s("*"), r(edge), s(if forward { ".T." } else { ".F." })])
    };
    for t in rotated(&a.steps, va)? {
        items.push(oe(st, t.edge, t.forward));
    }
    items.push(oe(st, seam, true));
    if !b.steps.is_empty() {
        for t in rotated(&b.steps, vb)? {
            items.push(oe(st, t.edge, t.forward));
        }
    }
    items.push(oe(st, seam, false));
    let lp = st.add("EDGE_LOOP", vec![s("''"), Arg::L(items.into_iter().map(r).collect())]);
    let bound = st.add("FACE_OUTER_BOUND", vec![s("''"), r(lp), s(".T.")]);
    let old = [format!("#{}", a.bound), format!("#{}", b.bound)];
    let mut kept: Vec<Arg> = face.a(1)?.list()?.iter().filter(|x| !old.iter().any(|o| x.is(o))).cloned().collect();
    kept.push(r(bound));
    let new_args = vec![face.a(0)?.clone(), Arg::L(kept), face.a(2)?.clone(), face.a(3)?.clone()];
    st.set(fid, new_args)?;
    for lp in [&a, &b] {
        let loop_id = st.st.e(lp.bound)?.a(1)?.r()?;
        st.drop(lp.bound)?;
        let le = st.st.e(loop_id)?;
        let oes: Vec<i64> = if le.ty == "EDGE_LOOP" { le.a(1)?.list()?.iter().map(Arg::r).collect::<Result<_>>()? } else { Vec::new() };
        st.drop(loop_id)?;
        for o in oes {
            st.drop(o)?;
        }
    }
    Ok(true)
}

// --------------------------------------------------------------------------- trimmed curves

type Hom = [f64; 4];

/// Insert the knot t once (Boehm); pw are homogeneous control points.
fn insert(p: i64, u: &[f64], pw: &[Hom], t: f64) -> Result<(Vec<f64>, Vec<Hom>)> {
    let n = pw.len() as i64 - 1;
    let mut k = p;
    while k < n && t >= pyget(u, k + 1)? {
        k += 1;
    }
    let mut q = Vec::new();
    for i in 0..n + 2 {
        if i <= k - p {
            q.push(pyget(pw, i)?);
        } else if i > k {
            q.push(pyget(pw, i - 1)?);
        } else {
            let ui = pyget(u, i)?;
            let a = div(t - ui, pyget(u, i + p)? - ui)?;
            let (prev, cur) = (pyget(pw, i - 1)?, pyget(pw, i)?);
            q.push([0, 1, 2, 3].map(|m| (1.0 - a) * prev[m] + a * cur[m]));
        }
    }
    let mut nu = pyslice(u, 0, k + 1);
    nu.push(t);
    nu.extend(pyslice(u, k + 1, u.len() as i64));
    Ok((nu, q))
}

/// The knots and control points of the stretch [a, b] of a B-spline.
fn piece(p: i64, u: Vec<f64>, pw: Vec<Hom>, a: f64, b: f64) -> Result<(Vec<f64>, Vec<Hom>)> {
    let (mut u, mut pw) = (u, pw);
    let eps = 1e-12 * pymax(1.0, (pyget(&u, -1)? - pyget(&u, 0)?).abs());
    let mut ends = Vec::new();
    for t in [a, b] {
        let same: Vec<f64> = u.iter().copied().filter(|x| (x - t).abs() <= eps).collect();
        let t = same.first().copied().unwrap_or(t);
        for _ in 0..(p - same.len() as i64).max(0) {
            (u, pw) = insert(p, &u, &pw, t)?;
        }
        ends.push(t);
    }
    let (a, b) = (ends[0], ends[1]);
    let empty = || py_err(PyKind::Value, "max() iterable argument is empty");
    let la = u.iter().rposition(|x| *x == a).ok_or_else(empty)? as i64;
    let fb = u.iter().position(|x| *x == b).ok_or_else(empty)? as i64;
    let mut knots = vec![a; (p + 1) as usize];
    knots.extend(pyslice(&u, la + 1, fb));
    knots.extend(vec![b; (p + 1) as usize]);
    Ok((knots, pyslice(&pw, la - p, fb)))
}

fn trim(st: &mut Edit, ec_id: i64, curves: &mut Curves) -> Result<bool> {
    let ec = st.st.e(ec_id)?.clone();
    let (v1, v2) = (ec.a(1)?.r()?, ec.a(2)?.r()?);
    let cid = ec.a(3)?.r()?;
    let c = curve(st, cid, curves)?;
    if v1 == v2 || !matches!(c.kind, Kind::Spline { .. }) {
        return Ok(false);
    }
    let (p1, p2) = (st.st.vertex(v1)?, st.st.vertex(v2)?);
    let (t1, t2) = (c.param(p1)?, c.param(p2)?);
    let along = ec.a(4)?.is(".T.");
    let (lo, hi) = c.domain()?;
    let span = hi - lo;
    if (t1 > t2) == along || (t2 - t1).abs() < 1e-9 * span {
        return Ok(false); // runs through the closing point of a closed curve, or is a point
    }
    let (a, b) = (pymin(t1, t2), pymax(t1, t2));
    if a - lo < 1e-9 * span && hi - b < 1e-9 * span {
        return Ok(false); // already the whole curve
    }
    let Kind::Spline { p, ctrl, w, u } = &c.kind else { return Ok(false) };
    let p = *p;
    let pw: Vec<Hom> = ctrl.iter().zip(w).map(|(q, w)| {
        let m = mul(*q, *w);
        [m[0], m[1], m[2], *w]
    }).collect();
    let (knots, mut cp) = piece(p, u.clone(), pw, a, b)?;
    let (first, last) = if along { (p1, p2) } else { (p2, p1) };
    let w0 = cp.first().ok_or_else(index_err)?[3];
    let m0 = mul(first, w0);
    cp[0] = [m0[0], m0[1], m0[2], w0]; // the ends exactly on the vertices
    let n = cp.len() - 1;
    let wl = cp[n][3];
    let ml = mul(last, wl);
    cp[n] = [ml[0], ml[1], ml[2], wl];
    let mut pts = Vec::new();
    for q in &cp {
        let point = [div(q[0], q[3])?, div(q[1], q[3])?, div(q[2], q[3])?];
        pts.push(r(st.point_entity(point)));
    }
    let (mut mults, mut values): (Vec<i64>, Vec<f64>) = (Vec::new(), Vec::new());
    for k in knots {
        if values.last() == Some(&k) {
            if let Some(m) = mults.last_mut() {
                *m += 1;
            }
        } else {
            values.push(k);
            mults.push(1);
        }
    }
    let m = Arg::L(mults.iter().map(|x| s(x.to_string())).collect());
    let v = Arg::L(values.iter().map(|x| s(py_repr(*x))).collect());
    let weights: Vec<f64> = cp.iter().map(|q| q[3]).collect();
    let pts = Arg::L(pts);
    let new = if weights.iter().all(|w| (w - 1.0).abs() < 1e-12) {
        st.add(
            "B_SPLINE_CURVE_WITH_KNOTS",
            vec![s("''"), s(p.to_string()), pts, s(".UNSPECIFIED."), s(".F."), s(".F."), m, v, s(".UNSPECIFIED.")],
        )
    } else {
        let w = Arg::L(weights.iter().map(|x| s(py_repr(*x))).collect());
        st.add_complex(format!(
            "( BOUNDED_CURVE() B_SPLINE_CURVE({p},{},.UNSPECIFIED.,.F.,.F.) B_SPLINE_CURVE_WITH_KNOTS({},{},.UNSPECIFIED.) CURVE() \
             GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE({}) REPRESENTATION_ITEM('') )",
            fmt(&pts),
            fmt(&m),
            fmt(&v),
            fmt(&w)
        ))
    };
    st.set(ec_id, vec![ec.a(0)?.clone(), ec.a(1)?.clone(), ec.a(2)?.clone(), r(new), ec.a(4)?.clone()])?;
    let old = st.st.get(cid).cloned();
    if let Some(old) = old {
        if (old.ty == "SURFACE_CURVE" || old.ty == "SEAM_CURVE") && st.users.get(&cid).is_none_or(|u| u.is_empty()) {
            let pcurves: Vec<i64> = match old.a(2)? {
                Arg::L(l) => l.iter().map(Arg::r).collect::<Result<_>>()?,
                Arg::S(_) => Vec::new(),
            };
            st.drop(cid)?;
            for pc in pcurves {
                let rep = match st.st.get(pc) {
                    Some(e) => Some(e.a(2)?.r()?),
                    None => None,
                };
                st.drop(pc)?;
                if let Some(rep) = rep {
                    if st.st.ent.contains_key(&rep) {
                        st.drop(rep)?;
                    }
                }
            }
        }
    }
    Ok(true)
}

// --------------------------------------------------------------------------- touching vertices

/// Give each fan of faces round a vertex its own vertex.
///
/// Where two parts of a solid touch at a point or along a line, the source CAD keeps one vertex
/// with two separate fans of faces round it (and two edges along the line). SolidCraft then takes
/// the solid for open; a vertex per fan, at the same point, is the manifold way to say it.
fn split_fans(st: &mut Edit) -> Result<usize> {
    // vertex -> (edge in, edge out) at each corner
    let mut corners: indexmap::IndexMap<i64, Vec<(i64, i64)>> = indexmap::IndexMap::new();
    let loops: Vec<i64> = st.st.ent.iter().filter(|(_, e)| e.ty == "EDGE_LOOP").map(|(i, _)| *i).collect();
    for lid in loops {
        let e = st.st.e(lid)?;
        let oes: Vec<&Entity> = e.a(1)?.list()?.iter().map(|o| st.st.e(o.r()?)).collect::<Result<_>>()?;
        for (i, cur) in oes.iter().enumerate() {
            let nxt = oes[(i + 1) % oes.len()];
            let edge = st.st.e(cur.a(3)?.r()?)?;
            let v = if cur.a(4)?.is(".T.") { edge.a(2)? } else { edge.a(1)? }.r()?;
            corners.entry(v).or_default().push((cur.a(3)?.r()?, nxt.a(3)?.r()?));
        }
    }
    let mut done = 0;
    for (v, pairs) in corners {
        let mut links: indexmap::IndexMap<i64, PySet> = indexmap::IndexMap::new();
        for (a, b) in pairs {
            links.entry(a).or_default().add(b);
            links.entry(b).or_default().add(a);
        }
        let mut fans: Vec<PySet> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for &start in links.keys() {
            if seen.contains(&start) {
                continue;
            }
            let (mut fan, mut todo) = (PySet::default(), vec![start]);
            while let Some(x) = todo.pop() {
                if !fan.contains(x) {
                    fan.add(x);
                    todo.extend(links[&x].items());
                }
            }
            seen.extend(fan.items());
            fans.push(fan);
        }
        for fan in fans.iter().skip(1) {
            let p = st.st.vertex(v)?;
            let pt = st.point_entity(p);
            let twin = st.add("VERTEX_POINT", vec![s("''"), r(pt)]);
            let me = format!("#{v}");
            for ec in fan.items() {
                let e = st.st.e(ec)?.clone();
                let mut args = vec![e.a(0)?.clone()];
                args.extend(e.args.iter().skip(1).take(2).map(|a| if a.is(&me) { r(twin) } else { a.clone() }));
                args.extend(e.args.iter().skip(3).cloned());
                st.set(ec, args)?;
            }
            done += 1;
        }
    }
    Ok(done)
}

// --------------------------------------------------------------------------- outer shells

/// The diagonal of the box round a shell's edges (points along them, not only vertices: a
/// circle has one vertex).
fn shell_size(st: &Edit, shell: i64, curves: &mut Curves) -> Result<f64> {
    let mut pts: Vec<V> = Vec::new();
    for f in st.st.e(shell)?.a(1)?.list()? {
        for b in st.st.e(f.r()?)?.a(1)?.list()? {
            let lp = st.st.e(st.st.e(b.r()?)?.a(1)?.r()?)?;
            if lp.ty == "VERTEX_LOOP" {
                pts.push(st.st.vertex(lp.a(1)?.r()?)?);
                continue;
            }
            for oe in lp.a(1)?.list()? {
                let edge = st.st.e(oe.r()?)?.a(3)?.r()?;
                if let Some(t) = travel(st, edge, true, curves)? {
                    pts.extend(t.points);
                }
            }
        }
    }
    if pts.is_empty() {
        return Ok(0.0);
    }
    let lo: Vec<f64> = (0..3).map(|k| pts.iter().map(|p| p[k]).fold(f64::INFINITY, pymin_f)).collect();
    let hi: Vec<f64> = (0..3).map(|k| pts.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, pymax_f)).collect();
    Ok(((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt())
}

fn pymin_f(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}
fn pymax_f(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

/// A solid with voids must have its largest shell as the outer one (cadmpeg may list a void).
fn outer_shell(st: &mut Edit, brep: i64, curves: &mut Curves) -> Result<bool> {
    let e = st.st.e(brep)?.clone(); // BREP_WITH_VOIDS('', outer, (oriented shells))
    let outer = e.a(1)?.r()?;
    let voids: Vec<i64> = e.a(2)?.list()?.iter().map(Arg::r).collect::<Result<_>>()?;
    let mut best: Option<(i64, f64)> = None;
    for v in voids {
        let size = shell_size(st, st.st.e(v)?.a(2)?.r()?, curves)?;
        if best.is_none_or(|(_, b)| size > b) {
            best = Some((v, size));
        }
    }
    let (best, _) = best.ok_or_else(|| py_err(PyKind::Value, "max() iterable argument is empty"))?;
    let inner = st.st.e(best)?.a(2)?.r()?;
    if shell_size(st, inner, curves)? <= shell_size(st, outer, curves)? {
        return Ok(false);
    }
    let o = st.st.e(best)?.clone(); // ORIENTED_CLOSED_SHELL('', *, shell, .F.)
    st.set(best, vec![o.a(0)?.clone(), o.a(1)?.clone(), r(outer), o.a(3)?.clone()])?;
    st.set(brep, vec![e.a(0)?.clone(), r(inner), e.a(2)?.clone()])?;
    Ok(true)
}

/// How many shells, seams and edges the clean-up changed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Done {
    pub outer_shells: usize,
    pub touching_vertices: usize,
    pub seams: usize,
    pub trimmed: usize,
}

/// Run one step; the kinds of failure Python's `except` lists skip it.
fn tolerate(r: Result<bool>, caught: &[PyKind]) -> Result<usize> {
    match r {
        Ok(b) => Ok(usize::from(b)),
        Err(e) if kind_of(&e).is_some_and(|k| caught.contains(&k)) => Ok(0),
        Err(e) => Err(e),
    }
}

/// The STEP text as Fusion's export would write it, and how many faces and edges changed.
pub fn clean(text: &str, tolerance_mm: f64) -> Result<(String, Done)> {
    use PyKind::*;
    let mut st = Edit::new(text)?;
    let mut curves: Curves = HashMap::new();
    let mut done = Done::default();
    let of = |st: &Edit, ty: &str| -> Vec<i64> { st.st.ent.iter().filter(|(_, e)| e.ty == ty).map(|(i, _)| *i).collect() };
    for brep in of(&st, "BREP_WITH_VOIDS") {
        done.outer_shells += tolerate(outer_shell(&mut st, brep, &mut curves), &[Key, Value, Index])?;
    }
    match split_fans(&mut st) {
        Ok(n) => done.touching_vertices = n,
        Err(e) if kind_of(&e).is_some_and(|k| [Key, Value, Index].contains(&k)) => {}
        Err(e) => return Err(e),
    }
    for fid in of(&st, "ADVANCED_FACE") {
        done.seams += tolerate(cut_open(&mut st, fid, &mut curves), &[Key, Value, ZeroDivision, StopIteration, Index])?;
    }
    for ec in of(&st, "EDGE_CURVE") {
        done.trimmed += tolerate(trim(&mut st, ec, &mut curves), &[Key, Value, ZeroDivision, Index])?;
    }
    let out = if done.outer_shells + done.touching_vertices + done.seams + done.trimmed > 0 { st.text()? } else { text.to_owned() };
    Ok((set_tolerance(&out, tolerance_mm), done))
}

static UNCERTAINTY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(UNCERTAINTY_MEASURE_WITH_UNIT\s*\(\s*LENGTH_MEASURE\s*\()[^)]*\)").expect("tolerance pattern"));

/// Declare the model's distance tolerance (readers check vertices on curves against it).
pub fn set_tolerance(text: &str, mm: f64) -> String {
    UNCERTAINTY.replacen(text, 1, |c: &regex::Captures| format!("{}{})", &c[1], py_repr(mm))).into_owned()
}

//! A STEP (ISO 10303-21) file as the repairs read it: its statements and the entities in them.
//!
//! Port of `step_fix.Step`. Every accessor fails where the Python one raises, so the repairs
//! give up (or carry on) exactly where the Python ones do.

pub mod clean;
pub mod edit;
pub mod fix;
pub mod mesh_fix;
pub mod split;

use std::sync::LazyLock;

use anyhow::Result;
use indexmap::IndexMap;
use regex::Regex;

pub type V = [f64; 3];

pub fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn mul(a: V, k: f64) -> V {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn norm(a: V) -> f64 {
    dot(a, a).sqrt()
}
pub fn unit(a: V) -> Option<V> {
    let n = norm(a);
    if n < 1e-15 { None } else { Some([a[0] / n, a[1] / n, a[2] / n]) }
}
pub fn mean(pts: &[V]) -> Option<V> {
    if pts.is_empty() {
        return None;
    }
    let mut c = [0.0; 3];
    for p in pts {
        c = add(c, *p);
    }
    Some(mul(c, 1.0 / pts.len() as f64))
}

/// The Python exception an operation raises: the STEP clean-up catches some kinds and lets the
/// others through, as `step_clean.py` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PyKind {
    Key,
    Value,
    Index,
    ZeroDivision,
    StopIteration,
    Type,
}

#[derive(Debug)]
pub struct PyErr {
    pub kind: PyKind,
    pub msg: String,
}

impl std::fmt::Display for PyErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for PyErr {}

pub fn py_err(kind: PyKind, msg: impl Into<String>) -> anyhow::Error {
    PyErr { kind, msg: msg.into() }.into()
}

/// The kind of Python exception an error stands for (None: any other failure).
pub fn kind_of(e: &anyhow::Error) -> Option<PyKind> {
    e.downcast_ref::<PyErr>().map(|p| p.kind)
}

/// `a / b` for floats, raising ZeroDivisionError as Python does.
pub fn div(a: f64, b: f64) -> Result<f64> {
    if b == 0.0 { Err(py_err(PyKind::ZeroDivision, "float division by zero")) } else { Ok(a / b) }
}

/// Python's float `%` for a positive divisor.
pub fn py_mod(x: f64, y: f64) -> f64 {
    let r = x % y;
    if r != 0.0 && (r < 0.0) != (y < 0.0) { r + y } else if r == 0.0 { 0.0_f64.copysign(y) } else { r }
}

/// A STEP parameter: a token as written, or a nested list.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    S(String),
    L(Vec<Arg>),
}

impl Arg {
    pub fn s(&self) -> Result<&str> {
        match self {
            Arg::S(s) => Ok(s),
            Arg::L(_) => Err(py_err(PyKind::Type, "'list' object has no attribute 'startswith'")),
        }
    }
    pub fn list(&self) -> Result<&[Arg]> {
        match self {
            Arg::L(l) => Ok(l),
            // Python iterates the characters and fails on the first `_ref`.
            Arg::S(_) => Err(py_err(PyKind::Value, "invalid literal for int() with base 10: ''")),
        }
    }
    /// `arg == ".T."` (a list is never equal to a token).
    pub fn is(&self, token: &str) -> bool {
        matches!(self, Arg::S(s) if s == token)
    }
    /// `len(arg)`: characters of a token, items of a list.
    pub fn py_len(&self) -> usize {
        match self {
            Arg::S(s) => s.chars().count(),
            Arg::L(l) => l.len(),
        }
    }
    /// `float(arg)`.
    pub fn f(&self) -> Result<f64> {
        let Arg::S(t) = self else { return Err(py_err(PyKind::Type, "float() argument must be a string or a real number, not 'list'")) };
        let t = t.trim();
        t.parse::<f64>().map_err(|_| py_err(PyKind::Value, format!("could not convert string to float: '{t}'")))
    }
    /// `int(arg)`.
    pub fn i(&self) -> Result<i64> {
        let Arg::S(t) = self else { return Err(py_err(PyKind::Type, "int() argument must be a string, not 'list'")) };
        let t = t.trim();
        t.parse::<i64>().map_err(|_| py_err(PyKind::Value, format!("invalid literal for int() with base 10: '{t}'")))
    }
    /// `_ref(arg)`: the entity id of `#123`.
    pub fn r(&self) -> Result<i64> {
        let Arg::S(t) = self else { return Err(py_err(PyKind::Type, "int() argument must be a string, not 'list'")) };
        let rest: String = t.chars().skip(1).collect();
        rest.trim().parse::<i64>().map_err(|_| py_err(PyKind::Value, format!("invalid literal for int() with base 10: '{rest}'")))
    }
}

/// Index into a parameter list, failing as Python's `IndexError` does.
pub fn at(args: &[Arg], i: usize) -> Result<&Arg> {
    args.get(i).ok_or_else(|| py_err(PyKind::Index, "list index out of range"))
}

/// Python's negative index `args[-1]`.
pub fn last(args: &[Arg]) -> Result<&Arg> {
    args.last().ok_or_else(|| py_err(PyKind::Index, "list index out of range"))
}

#[derive(Clone, Debug)]
pub struct Entity {
    pub ty: String,
    pub args: Vec<Arg>,
    pub line: usize,
}

impl Entity {
    pub fn a(&self, i: usize) -> Result<&Arg> {
        at(&self.args, i)
    }
}

/// `_split_args`: a STEP parameter list as nested lists of tokens.
pub fn split_args(text: &str) -> Result<Vec<Arg>> {
    let chars: Vec<char> = text.chars().collect();
    // The stack of open lists; Python pops its root on an unbalanced ')' and then fails on the
    // next append, which `dead` stands for.
    let mut stack: Vec<Vec<Arg>> = vec![Vec::new()];
    let mut dead = false;
    let mut tok = String::new();
    let mut i = 0;
    let push = |stack: &mut Vec<Vec<Arg>>, dead: bool, a: Arg| -> Result<()> {
        if dead {
            return Err(py_err(PyKind::Index, "pop from empty list"));
        }
        stack.last_mut().ok_or_else(|| py_err(PyKind::Index, "list index out of range"))?.push(a);
        Ok(())
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            let mut j = i + 1;
            while j < chars.len() {
                if chars[j] == '\'' {
                    if j + 1 < chars.len() && chars[j + 1] == '\'' {
                        j += 2;
                        continue;
                    }
                    break;
                }
                j += 1;
            }
            let end = (j + 1).min(chars.len());
            tok.extend(&chars[i..end]);
            i = j + 1;
            continue;
        }
        match c {
            '(' => {
                if dead {
                    return Err(py_err(PyKind::Index, "pop from empty list"));
                }
                stack.push(Vec::new());
                tok.clear();
            }
            ')' => {
                let t = tok.trim();
                if !t.is_empty() {
                    push(&mut stack, dead, Arg::S(t.to_owned()))?;
                }
                tok.clear();
                if dead {
                    return Err(py_err(PyKind::Index, "pop from empty list"));
                }
                if stack.len() > 1 {
                    let done = stack.pop().unwrap_or_default();
                    push(&mut stack, dead, Arg::L(done))?;
                } else {
                    dead = true;
                }
            }
            ',' => {
                let t = tok.trim();
                if !t.is_empty() {
                    push(&mut stack, dead, Arg::S(t.to_owned()))?;
                }
                tok.clear();
            }
            _ => tok.push(c),
        }
        i += 1;
    }
    let t = tok.trim();
    if !t.is_empty() {
        push(&mut stack, dead, Arg::S(t.to_owned()))?;
    }
    // Lists still open at the end sit where they were opened, as in Python. After an extra ')'
    // only the root is left: Python returns it as it was.
    while stack.len() > 1 {
        let done = stack.pop().unwrap_or_default();
        stack.last_mut().ok_or_else(|| py_err(PyKind::Index, "list index out of range"))?.push(Arg::L(done));
    }
    Ok(stack.pop().unwrap_or_default())
}

static ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)#(\d+)\s*=\s*([A-Z_0-9]+)\s*\((.*)\)\s*;\s*$").expect("entity pattern"));
static LAST_FLAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r",\s*\.[TF]\.\s*\)\s*;$").expect("flag pattern"));

pub struct Step {
    pub statements: Vec<String>,
    pub ent: IndexMap<i64, Entity>,
}

pub const REVOLVED: [&str; 2] = ["CYLINDRICAL_SURFACE", "CONICAL_SURFACE"];

impl Step {
    pub fn new(text: &str) -> Step {
        // Statements end with ';' (the strings in these files hold none).
        let mut statements = Vec::new();
        let mut start = 0;
        for (i, b) in text.bytes().enumerate() {
            if b == b';' {
                statements.push(text[start..=i].to_owned());
                start = i + 1;
            }
        }
        statements.push(text[start..].to_owned());
        let mut ent = IndexMap::new();
        for (k, st) in statements.iter().enumerate() {
            if let Some(m) = ENTITY.captures(st.trim()) {
                let Ok(id) = m[1].parse::<i64>() else { continue };
                if let Ok(args) = split_args(&m[3]) {
                    ent.insert(id, Entity { ty: m[2].to_owned(), args, line: k });
                }
            }
        }
        Step { statements, ent }
    }

    pub fn text(&self) -> String {
        self.statements.concat()
    }

    /// `self.ent[i]`.
    pub fn e(&self, i: i64) -> Result<&Entity> {
        self.ent.get(&i).ok_or_else(|| py_err(PyKind::Key, i.to_string()))
    }

    /// `self.ent.get(i)`.
    pub fn get(&self, i: i64) -> Option<&Entity> {
        self.ent.get(&i)
    }

    pub fn point(&self, i: i64) -> Result<V> {
        let c = self.e(i)?.a(1)?.list()?;
        Ok([at(c, 0)?.f()?, at(c, 1)?.f()?, if c.len() > 2 { at(c, 2)?.f()? } else { 0.0 }])
    }

    pub fn direction(&self, i: i64) -> Result<V> {
        let c = self.e(i)?.a(1)?.list()?;
        let v = [at(c, 0)?.f()?, at(c, 1)?.f()?, if c.len() > 2 { at(c, 2)?.f()? } else { 0.0 }];
        Ok(unit(v).unwrap_or([0.0, 0.0, 1.0]))
    }

    /// Origin, x, y, z of an AXIS2_PLACEMENT_3D.
    pub fn placement(&self, i: i64) -> Result<(V, V, V, V)> {
        let e = self.e(i)?;
        let o = self.point(e.a(1)?.r()?)?;
        let z = if e.a(2)?.s()?.starts_with('#') { self.direction(e.a(2)?.r()?)? } else { [0.0, 0.0, 1.0] };
        let xr = if e.args.len() > 3 && e.a(3)?.s()?.starts_with('#') {
            self.direction(e.a(3)?.r()?)?
        } else if z[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let x = unit(sub(xr, mul(z, dot(xr, z)))).unwrap_or([1.0, 0.0, 0.0]);
        Ok((o, x, cross(z, x), z))
    }

    pub fn vertex(&self, i: i64) -> Result<V> {
        self.point(self.e(i)?.a(1)?.r()?)
    }

    pub fn surface(&self, face: &Entity) -> Result<Option<&Entity>> {
        Ok(self.get(face.a(2)?.r()?))
    }

    /// The surface's own normal at p (before the face's same_sense flag).
    pub fn surface_normal(&self, surf: &Entity, p: V) -> Result<Option<V>> {
        let t = surf.ty.as_str();
        if t == "PLANE" {
            return Ok(Some(self.placement(surf.a(1)?.r()?)?.3));
        }
        if matches!(t, "CYLINDRICAL_SURFACE" | "CONICAL_SURFACE" | "SPHERICAL_SURFACE" | "TOROIDAL_SURFACE") {
            let (o, _x, _y, z) = self.placement(surf.a(1)?.r()?)?;
            let d = sub(p, o);
            let r = unit(sub(d, mul(z, dot(d, z))));
            if t == "SPHERICAL_SURFACE" {
                return Ok(unit(d));
            }
            let Some(r) = r else { return Ok(None) };
            if t == "CYLINDRICAL_SURFACE" {
                return Ok(Some(r));
            }
            if t == "CONICAL_SURFACE" {
                let mut a = surf.a(3)?.f()?;
                if a.abs() > std::f64::consts::PI / 2.0 {
                    a = a.to_radians(); // the file's angles are in degrees
                }
                return Ok(unit(sub(mul(r, a.cos()), mul(z, a.sin()))));
            }
            return Ok(unit(sub(d, mul(r, surf.a(2)?.f()?)))); // torus
        }
        Ok(None)
    }

    pub fn curve(&self, cid: i64) -> Result<Option<&Entity>> {
        let mut e = self.get(cid);
        while let Some(x) = e {
            if x.ty == "SURFACE_CURVE" || x.ty == "SEAM_CURVE" {
                e = self.get(x.a(1)?.r()?);
            } else {
                break;
            }
        }
        Ok(e)
    }

    /// Points along a curve from a to b in the curve's own direction.
    pub fn curve_samples(&self, cid: i64, a: V, b: V, closed: bool) -> Result<Option<Vec<V>>> {
        let Some(e) = self.curve(cid)? else { return Ok(None) };
        if e.ty == "LINE" {
            return Ok(Some(vec![a, b]));
        }
        if e.ty == "CIRCLE" || e.ty == "ELLIPSE" {
            let (o, x, y, _z) = self.placement(e.a(1)?.r()?)?;
            let ang = |p: V| {
                let d = sub(p, o);
                dot(d, y).atan2(dot(d, x))
            };
            let rx = e.a(2)?.f()?;
            let ry = if e.ty == "CIRCLE" { rx } else { e.a(3)?.f()? };
            let t0 = ang(a);
            let mut span = py_mod(ang(b) - t0, 2.0 * std::f64::consts::PI);
            if closed || span < 1e-9 {
                span = 2.0 * std::f64::consts::PI;
            }
            let n = 16.0;
            return Ok(Some(
                (0..=16)
                    .map(|k| {
                        let t = t0 + span * k as f64 / n;
                        add(o, add(mul(x, rx * t.cos()), mul(y, ry * t.sin())))
                    })
                    .collect(),
            ));
        }
        if e.ty.starts_with("B_SPLINE_CURVE") {
            let pts: Result<Vec<V>> = (|| e.a(2)?.list()?.iter().map(|r| self.point(r.r()?)).collect())();
            return Ok(pts.ok());
        }
        Ok(None)
    }

    /// Rewrite an entity's last argument (.T./.F.); true when it changed.
    pub fn set_last_flag(&mut self, id: i64, value: bool) -> Result<bool> {
        let flag = if value { ".T." } else { ".F." };
        let line = self.e(id)?.line;
        let stmt = &self.statements[line];
        let body = stmt.trim_end();
        if body.ends_with(&format!(",{flag});")) {
            return Ok(false);
        }
        let new_body = LAST_FLAG.replace(body, format!(",{flag});").as_str()).into_owned();
        if new_body == body {
            return Ok(false);
        }
        let rest = stmt[body.len()..].to_owned();
        self.statements[line] = new_body + &rest;
        if let Some(e) = self.ent.get_mut(&id)
            && let Some(a) = e.args.last_mut() {
                *a = Arg::S(flag.to_owned());
            }
        Ok(true)
    }

    /// Rewrite the last flag of a statement without touching the parsed entity (step_fix does
    /// not update it).
    pub fn set_statement_flag(&mut self, id: i64, flag: &str) -> Result<bool> {
        let line = self.e(id)?.line;
        let stmt = &self.statements[line];
        let body = stmt.trim_end();
        let new_body = LAST_FLAG.replace(body, format!(",{flag});").as_str()).into_owned();
        if new_body == body {
            return Ok(false);
        }
        let rest = stmt[body.len()..].to_owned();
        self.statements[line] = new_body + &rest;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_nested_lists_and_strings() {
        let a = split_args("'a,(b)',(#1,#2),#3,.T.").unwrap();
        assert_eq!(
            a,
            vec![
                Arg::S("'a,(b)'".into()),
                Arg::L(vec![Arg::S("#1".into()), Arg::S("#2".into())]),
                Arg::S("#3".into()),
                Arg::S(".T.".into())
            ]
        );
        let typed = split_args("'',LENGTH_MEASURE(1.5)").unwrap();
        assert_eq!(typed, vec![Arg::S("''".into()), Arg::L(vec![Arg::S("1.5".into())])]);
    }

    #[test]
    fn reads_points() {
        let st = Step::new("DATA;\n#1=CARTESIAN_POINT('',(1.,2.5,-3.E-1));\n#2=VERTEX_POINT('',#1);\nENDSEC;");
        assert_eq!(st.vertex(2).unwrap(), [1.0, 2.5, -0.3]);
        assert_eq!(st.text(), "DATA;\n#1=CARTESIAN_POINT('',(1.,2.5,-3.E-1));\n#2=VERTEX_POINT('',#1);\nENDSEC;");
    }
}

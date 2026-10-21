//! A STEP file that can be edited: entities added, rewritten and dropped, as `step_clean._Edit`.
//!
//! New entities get the next free ids and are written before `ENDSEC;`. Every entity knows
//! which others refer to it (`users`), in a set that iterates as CPython's does, because the
//! clean-up creates entities in that order.

use std::collections::HashMap;
use std::sync::LazyLock;

use anyhow::Result;
use indexmap::IndexMap;
use regex::Regex;

use super::{Arg, Entity, PyKind, Step, V, py_err, split_args};
use crate::pyset::PySet;
use crate::util::py_repr;

/// The line of an entity that is not in the file yet.
pub const NEW: usize = usize::MAX;

static COMPLEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)#(\d+)\s*=\s*\((.*)\)\s*;\s*$").expect("complex pattern"));
static PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*([A-Z_0-9]+)\s*\(").expect("part pattern"));

/// A parameter list written back as STEP: `(a,(b,c))`.
pub fn fmt(a: &Arg) -> String {
    match a {
        Arg::S(s) => s.clone(),
        Arg::L(l) => format!("({})", l.iter().map(fmt).collect::<Vec<_>>().join(",")),
    }
}

pub fn fmt_args(args: &[Arg]) -> String {
    format!("({})", args.iter().map(fmt).collect::<Vec<_>>().join(","))
}

pub fn s(text: impl Into<String>) -> Arg {
    Arg::S(text.into())
}

pub fn r(id: i64) -> Arg {
    Arg::S(format!("#{id}"))
}

/// The named parts of a complex entity: 'A(..) B(..)' -> {A: args, B: args}.
fn parts(text: &str) -> Result<IndexMap<String, Vec<Arg>>> {
    let b = text.as_bytes();
    let mut out = IndexMap::new();
    let mut i = 0;
    while i < b.len() {
        let Some(m) = PART.captures(&text[i..]) else { break };
        let name = m[1].to_owned();
        let start = i + m.get(0).map_or(0, |g| g.end());
        let (mut depth, mut j) = (1, start);
        while j < b.len() && depth > 0 {
            match b[j] {
                b'\'' => {
                    j = text[j + 1..].find('\'').map(|k| k + j + 1).ok_or_else(|| py_err(PyKind::Value, "substring not found"))?;
                }
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
        out.insert(name, split_args(&text[start..j.saturating_sub(1).max(start)])?);
        i = j;
    }
    Ok(out)
}

/// Every entity id an argument list refers to (`_refs`).
pub fn refs(args: &[Arg]) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Arg::L(l) => out.extend(refs(l)?),
            Arg::S(t) if t.starts_with('#') => out.push(a.r()?),
            Arg::S(_) => {}
        }
    }
    Ok(out)
}

pub struct Edit {
    pub st: Step,
    pub complex: HashMap<i64, IndexMap<String, Vec<Arg>>>,
    pub next_id: i64,
    pub new: Vec<i64>,
    pub raw: HashMap<i64, String>,
    pub users: HashMap<i64, PySet>,
}

impl Edit {
    pub fn new(text: &str) -> Result<Edit> {
        let st = Step::new(text);
        let mut complex = HashMap::new();
        for line in &st.statements {
            if let Some(m) = COMPLEX.captures(line.trim()) {
                let Ok(id) = m[1].parse::<i64>() else { continue };
                if !st.ent.contains_key(&id) {
                    complex.insert(id, parts(&m[2])?);
                }
            }
        }
        let next_id = st.ent.keys().chain(complex.keys()).copied().chain([0]).max().unwrap_or(0) + 1;
        let mut users: HashMap<i64, PySet> = HashMap::new();
        for (&eid, e) in &st.ent {
            for r in refs(&e.args)? {
                users.entry(r).or_default().add(eid);
            }
        }
        Ok(Edit { st, complex, next_id, new: Vec::new(), raw: HashMap::new(), users })
    }

    pub fn add(&mut self, ty: &str, args: Vec<Arg>) -> i64 {
        let eid = self.next_id;
        self.next_id += 1;
        let ids = refs(&args).unwrap_or_default();
        self.st.ent.insert(eid, Entity { ty: ty.to_owned(), args, line: NEW });
        self.new.push(eid);
        for r in ids {
            self.users.entry(r).or_default().add(eid);
        }
        eid
    }

    pub fn set(&mut self, eid: i64, args: Vec<Arg>) -> Result<()> {
        let old = refs(&self.st.e(eid)?.args)?;
        for r in old {
            if let Some(u) = self.users.get_mut(&r) {
                u.discard(eid);
            }
        }
        for r in refs(&args)? {
            self.users.entry(r).or_default().add(eid);
        }
        let e = self.st.ent.get_mut(&eid).ok_or_else(|| py_err(PyKind::Key, eid.to_string()))?;
        e.args = args;
        if e.line != NEW {
            let stmt = &self.st.statements[e.line];
            let lead: String = stmt.chars().take_while(|c| c.is_whitespace()).collect();
            self.st.statements[e.line] = format!("{lead}#{eid} = {}{};", e.ty, fmt_args(&e.args));
        }
        Ok(())
    }

    /// Remove an entity nothing refers to any more.
    pub fn drop(&mut self, eid: i64) -> Result<()> {
        if self.users.get(&eid).is_some_and(|u| !u.is_empty()) {
            return Ok(());
        }
        let e = self.st.ent.shift_remove(&eid).ok_or_else(|| py_err(PyKind::Key, eid.to_string()))?;
        for r in refs(&e.args)? {
            if let Some(u) = self.users.get_mut(&r) {
                u.discard(eid);
            }
        }
        if e.line != NEW {
            self.st.statements[e.line] = String::new();
        }
        Ok(())
    }

    /// A complex entity, written as given: '( A(..) B(..) )'.
    pub fn add_complex(&mut self, body: String) -> i64 {
        let eid = self.next_id;
        self.next_id += 1;
        for m in super::split::REF.captures_iter(&body) {
            if let Ok(r) = m[1].parse::<i64>() {
                self.users.entry(r).or_default().add(eid);
            }
        }
        self.raw.insert(eid, body);
        self.new.push(eid);
        eid
    }

    pub fn text(&self) -> Result<String> {
        let mut new = String::new();
        for i in &self.new {
            if let Some(raw) = self.raw.get(i) {
                new.push_str(&format!("\n#{i} = {raw};"));
            } else if let Some(e) = self.st.ent.get(i) {
                new.push_str(&format!("\n#{i} = {}{};", e.ty, fmt_args(&e.args)));
            }
        }
        let end = self
            .st
            .statements
            .iter()
            .rposition(|s| s.trim() == "ENDSEC;")
            .ok_or_else(|| py_err(PyKind::Value, "max() iterable argument is empty"))?;
        Ok(self.st.statements[..end].concat() + &new + &self.st.statements[end..].concat())
    }

    pub fn point_entity(&mut self, p: V) -> i64 {
        self.add("CARTESIAN_POINT", vec![s("''"), Arg::L(vec![s(py_repr(p[0])), s(py_repr(p[1])), s(py_repr(p[2]))])])
    }

    /// The members of an entity's users, in CPython's order (a snapshot).
    pub fn users_of(&self, eid: i64) -> Vec<i64> {
        self.users.get(&eid).map(PySet::items).unwrap_or_default()
    }
}

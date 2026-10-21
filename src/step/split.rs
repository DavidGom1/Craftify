//! Split a multi-body STEP file into files that keep only some of its solids. Port of
//! `step_split.py`.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use anyhow::Result;
use indexmap::IndexMap;
use regex::Regex;

use super::{PyKind, py_err};

pub static REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#(\d+)").expect("ref pattern"));
static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*#(\d+)\s*=\s*(\(?\s*[A-Z_0-9]+)").expect("head pattern"));
static POINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\(\s*([-+0-9.eE]+)\s*,\s*([-+0-9.eE]+)\s*,\s*([-+0-9.eE]+)\s*\)").expect("point pattern")
});
/// Every body, surfaces too.
const SOLIDS: [&str; 3] = ["MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS", "SHELL_BASED_SURFACE_MODEL"];
const STYLES: [&str; 2] = ["STYLED_ITEM", "OVER_RIDING_STYLED_ITEM"];

pub type Box3 = ([f64; 3], [f64; 3]);

#[derive(Clone, Debug)]
pub struct Solid {
    pub id: i64,
    /// From its vertices.
    pub bbox: Option<Box3>,
}

pub struct StepFile {
    statements: Vec<String>,
    line_of: HashMap<i64, usize>,
    pub kind: IndexMap<i64, String>,
    refs: IndexMap<i64, Vec<i64>>,
}

fn head_id(st: &str) -> Option<i64> {
    HEAD.captures(st).and_then(|m| m[1].parse().ok())
}

impl StepFile {
    pub fn new(text: &str) -> StepFile {
        let mut statements = Vec::new();
        let mut start = 0;
        for (i, b) in text.bytes().enumerate() {
            if b == b';' {
                statements.push(text[start..=i].to_owned());
                start = i + 1;
            }
        }
        statements.push(text[start..].to_owned());
        let (mut line_of, mut kind, mut refs) = (HashMap::new(), IndexMap::new(), IndexMap::new());
        for (k, st) in statements.iter().enumerate() {
            let Some(m) = HEAD.captures(st) else { continue };
            let Ok(eid) = m[1].parse::<i64>() else { continue };
            line_of.insert(eid, k);
            kind.insert(eid, m[2].trim_start_matches(['(', ' ']).trim().to_owned());
            let body = st.find('=').map_or("", |i| &st[i + 1..]);
            refs.insert(eid, REF.captures_iter(body).filter_map(|c| c[1].parse().ok()).collect());
        }
        StepFile { statements, line_of, kind, refs }
    }

    /// The bodies, by entity id: the order cadmpeg wrote them.
    pub fn solids(&self) -> Result<Vec<Solid>> {
        let mut ids: Vec<i64> = self.kind.keys().copied().collect();
        ids.sort_unstable();
        let mut out = Vec::new();
        for eid in ids {
            if SOLIDS.contains(&self.kind[&eid].as_str()) {
                out.push(Solid { id: eid, bbox: self.vertex_box(eid)? });
            }
        }
        Ok(out)
    }

    fn reach(&self, start: &[i64], blocked: &HashSet<i64>) -> HashSet<i64> {
        let mut seen = HashSet::new();
        let mut stack: Vec<i64> = start.iter().copied().filter(|e| !blocked.contains(e)).collect();
        while let Some(e) = stack.pop() {
            if seen.contains(&e) || blocked.contains(&e) {
                continue;
            }
            seen.insert(e);
            if let Some(r) = self.refs.get(&e) {
                stack.extend(r.iter().copied().filter(|x| !seen.contains(x) && !blocked.contains(x)));
            }
        }
        seen
    }

    fn vertex_box(&self, solid: i64) -> Result<Option<Box3>> {
        let mut pts = Vec::new();
        for e in self.reach(&[solid], &HashSet::new()) {
            if self.kind.get(&e).map(String::as_str) == Some("VERTEX_POINT") {
                if let Some(&first) = self.refs.get(&e).and_then(|r| r.first()) {
                    if let Some(p) = self.point(first)? {
                        pts.push(p);
                    }
                }
            }
        }
        let Some(first) = pts.first() else { return Ok(None) };
        let (mut lo, mut hi) = (*first, *first);
        for p in &pts {
            for k in 0..3 {
                if p[k] < lo[k] {
                    lo[k] = p[k];
                }
                if p[k] > hi[k] {
                    hi[k] = p[k];
                }
            }
        }
        Ok(Some((lo, hi)))
    }

    fn point(&self, eid: i64) -> Result<Option<[f64; 3]>> {
        let line = *self.line_of.get(&eid).ok_or_else(|| py_err(PyKind::Key, eid.to_string()))?;
        let Some(m) = POINT.captures(&self.statements[line]) else { return Ok(None) };
        let f = |k: usize| m[k].parse::<f64>().map_err(|_| py_err(PyKind::Value, format!("could not convert string to float: '{}'", &m[k])));
        Ok(Some([f(1)?, f(2)?, f(3)?]))
    }

    /// The file with every other body, and what only it used, taken out. Styles of what goes
    /// go too (a face colour would otherwise keep the face of another body in the file).
    pub fn keep_only(&self, keep: &HashSet<i64>) -> Result<String> {
        let drop: HashSet<i64> = self.solids()?.into_iter().map(|s| s.id).filter(|id| !keep.contains(id)).collect();
        let keep_list: Vec<i64> = keep.iter().copied().collect();
        let kept = self.reach(&keep_list, &HashSet::new());
        let drop_list: Vec<i64> = drop.iter().copied().collect();
        // used by the dropped bodies only
        let mut gone = self.reach(&drop_list, &kept);
        gone.extend(drop.iter().copied());
        let dead: HashSet<i64> = self
            .kind
            .iter()
            .filter(|(e, k)| STYLES.contains(&k.as_str()) && self.refs[*e].iter().any(|r| gone.contains(r)))
            .map(|(e, _)| *e)
            .collect();
        let remove: HashSet<i64> = gone.union(&dead).copied().collect();
        let mut statements = self.statements.clone();
        for (eid, refs) in &self.refs {
            if remove.contains(eid) {
                continue;
            }
            let mut hit: Vec<i64> = refs.iter().copied().filter(|r| remove.contains(r)).collect();
            hit.sort_unstable();
            hit.dedup();
            if hit.is_empty() {
                continue;
            }
            let line = self.line_of[eid];
            let mut st = statements[line].clone();
            for d in &hit {
                // out of item lists
                st = remove_first_item(&st, *d);
                st = remove_last_item(&st, *d);
            }
            statements[line] = st;
        }
        let referenced: HashSet<i64> = self.refs.values().flatten().copied().collect();
        let roots: Vec<i64> = self.refs.keys().copied().filter(|e| !referenced.contains(e) && !remove.contains(e)).collect();
        let live = self.reach(&roots, &remove);
        Ok(statements.into_iter().filter(|st| head_id(st).is_none_or(|id| live.contains(&id))).collect())
    }
}

/// `re.sub(rf"(?<=\()\s*#{d}\s*(?:,\s*)?(?=[#)])", "", st)`: the item right after a '(',
/// with the comma after it (the regex crate has no look-around).
fn remove_first_item(st: &str, d: i64) -> String {
    let pat = Regex::new(&format!(r"\(\s*#{d}")).expect("item pattern");
    let b = st.as_bytes();
    let ws = |mut i: usize| {
        while i < b.len() && (b[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let (mut out, mut copied, mut from) = (String::new(), 0, 0);
    while let Some(m) = pat.find_at(st, from) {
        let after = ws(m.end());
        // the optional comma first (greedy), then without it
        let with_comma = (after < b.len() && b[after] == b',').then(|| ws(after + 1));
        let end = [with_comma, Some(after)].into_iter().flatten().find(|&e| e < b.len() && (b[e] == b'#' || b[e] == b')'));
        match end {
            Some(e) => {
                out.push_str(&st[copied..m.start() + 1]); // keep the '('
                copied = e;
                from = e;
            }
            None => from = m.start() + 1,
        }
        if from >= st.len() {
            break;
        }
    }
    out.push_str(&st[copied..]);
    out
}

/// `re.sub(rf",\s*#{d}(?=\s*[),])", "", st)`: the item at the end of a list, or before another
/// one (the regex crate has no look-ahead).
fn remove_last_item(st: &str, d: i64) -> String {
    let pat = Regex::new(&format!(r",\s*#{d}")).expect("item pattern");
    let mut out = String::new();
    let mut copied = 0;
    let mut from = 0;
    while let Some(m) = pat.find_at(st, from) {
        let rest = &st[m.end()..];
        let next = rest.trim_start();
        if next.starts_with(')') || next.starts_with(',') {
            out.push_str(&st[copied..m.start()]);
            copied = m.end();
            from = m.end();
        } else {
            from = m.start() + 1;
        }
        if from > st.len() {
            break;
        }
    }
    out.push_str(&st[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::{remove_first_item, remove_last_item};

    #[test]
    fn removes_trailing_items_like_the_lookahead() {
        assert_eq!(remove_last_item("('',(#1,#5),#9)", 5), "('',(#1),#9)");
        assert_eq!(remove_last_item("(#1,#5,#7)", 5), "(#1,#7)");
        assert_eq!(remove_last_item("(#1,#55)", 5), "(#1,#55)");
        assert_eq!(remove_last_item("(#1, #5 )", 5), "(#1 )");
    }

    #[test]
    fn removes_leading_items_like_the_lookaround() {
        assert_eq!(remove_first_item("('',(#5,#7))", 5), "('',(#7))");
        assert_eq!(remove_first_item("(#5)", 5), "()");
        assert_eq!(remove_first_item("( #5 , #7)", 5), "(#7)");
        assert_eq!(remove_first_item("(#55,#7)", 5), "(#55,#7)");
        assert_eq!(remove_first_item("(#5,'x')", 5), "(#5,'x')");
    }
}

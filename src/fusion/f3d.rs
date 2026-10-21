//! Read a Fusion design (.f3d) without Fusion: its container and Design segment. Port of
//! `fusion/f3d.py`.
//!
//! Follows the format description cadmpeg publishes (docs/formats/f3d.md, CC BY 4.0). An .f3d
//! is a ZIP; the Design asset folder holds FusionDesignSegmentType1/MetaStream.dat (a type table
//! and record index) and BulkStream.dat (the records).

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::LazyLock;

use anyhow::{Result, anyhow, bail};
use indexmap::IndexMap;
use regex::bytes::Regex;
use serde_json::Value;

use crate::util::py_sum;

const MAX_ENTRY: u64 = 512 * 1024 * 1024;

pub const PLACEMENT: &str = "CE2913AA-CFE0-4F04-9102-24424ED3BCFA"; // occurrence placement (add-in Component)
pub const NAMING_SPACE_BASE: &str = "21F379C8-CAFD-4985-B461-767673A4C502"; // component naming space (base type)
pub const BODY_MAP: &str = "74CA4562-59F8-4C97-8AD1-F8297C21F9AA"; // BREP body map (add-in Body)
pub const TRANSFORM_CACHE: &str = "549FBB80-B890-473E-A5C0-415D3D9BF4E6"; // solved world transforms of an occurrence
pub const OCCURRENCE_TRANSFORM: &str = "66E375EA-40F1-4115-8D83-0F23DD30593E"; // an occurrence's transform object
pub const JOINT: &str = "8DFDD543-F823-43AA-BDAF-C638E023F0A0"; // the context Fusion shows a joint-driven occurrence in

/// Row-major 4x4, column vectors, mm.
pub type Mat4 = [f64; 16];
pub const IDENTITY4: Mat4 = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// A truncated or malformed file (Python's `F3dError`).
#[derive(Debug)]
pub struct F3dError(pub String);

impl std::fmt::Display for F3dError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for F3dError {}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn need(&self, n: usize) -> Result<()> {
        if self.pos + n > self.data.len() {
            return Err(F3dError(format!("truncated at {} (+{n})", self.pos)).into());
        }
        Ok(())
    }
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        self.need(n)?;
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into()?))
    }
    fn ascii(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        if n > 1 << 20 {
            return Err(F3dError(format!("string too long at {}", self.pos)).into());
        }
        Ok(self.take(n)?.iter().map(|b| *b as char).collect())
    }
    fn utf16(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        if n > 1 << 20 {
            return Err(F3dError(format!("string too long at {}", self.pos)).into());
        }
        let b = self.take(2 * n)?;
        let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Ok(String::from_utf16_lossy(&units))
    }
}

fn f64_at(b: &[u8], at: usize) -> f64 {
    f64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]))
}

pub struct TypeEntry {
    pub guid: String,
    pub base: String,
    pub ids: Vec<u64>,
}

pub struct Segment {
    pub types: Vec<TypeEntry>,
    /// Entity id -> BulkStream offset.
    pub index: IndexMap<u64, u64>,
    pub bulk: Vec<u8>,
    /// Entity id -> index into `types`.
    pub type_of: HashMap<u64, usize>,
    offsets: Vec<u64>,
}

impl Segment {
    /// The bytes of one record: from its offset to the next record's.
    pub fn record(&self, entity: u64) -> Result<&[u8]> {
        let start = *self.index.get(&entity).ok_or_else(|| anyhow!("KeyError: {entity}"))?;
        let next = self.offsets.partition_point(|o| *o <= start);
        let end = self.offsets.get(next).copied().unwrap_or(self.bulk.len() as u64);
        let (s, e) = (start as usize, end as usize);
        Ok(if s >= self.bulk.len() || e <= s { &[] } else { &self.bulk[s..e.min(self.bulk.len())] })
    }

    pub fn of_type(&self, guid: &str) -> &[u64] {
        self.types.iter().find(|t| t.guid.to_uppercase() == guid.to_uppercase()).map_or(&[], |t| t.ids.as_slice())
    }

    fn guid_of(&self, entity: u64) -> Option<&str> {
        self.type_of.get(&entity).map(|&i| self.types[i].guid.as_str())
    }
}

fn parse_meta(data: &[u8]) -> Result<Segment> {
    let mut r = Reader { data, pos: 0 };
    r.ascii()?; // name
    r.u32()?; // segment id
    r.utf16()?; // asset
    let magic = r.u32()?;
    for _ in 0..(if magic == 1234 { 3 } else { 1 }) {
        r.u32()?;
    }
    r.ascii()?; // full name
    r.ascii()?; // add-in
    r.u32()?; // kind
    r.u32()?;
    let mut types = Vec::new();
    for _ in 0..r.u32()? {
        let guid = r.ascii()?;
        let base = r.ascii()?;
        r.u32()?; // version
        r.ascii()?; // add-in
        let n = r.u32()?;
        let ids = (0..n).map(|_| r.u64()).collect::<Result<Vec<_>>>()?;
        types.push(TypeEntry { guid, base, ids });
    }
    for _ in 0..r.u32()? {
        r.u64()?; // named
    }
    let mut index = IndexMap::new();
    for _ in 0..r.u32()? {
        let e = r.u64()?;
        index.insert(e, r.u64()?);
    }
    for _ in 0..r.u32()? {
        r.u64()?; // secondary
        r.u64()?;
    }
    let mut type_of = HashMap::new();
    for (i, t) in types.iter().enumerate() {
        for e in &t.ids {
            type_of.insert(*e, i);
        }
    }
    let mut offsets: Vec<u64> = index.values().copied().collect();
    offsets.sort_unstable();
    Ok(Segment { types, index, bulk: Vec::new(), type_of, offsets })
}

pub struct F3d {
    pub design: Segment,
    pub redirections: Option<Value>,
}

/// An entry by its exact name (looked up by position: `by_name` does not find names such as
/// "FusionAssetName[Active]/...").
fn read_zip_entry(z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let index = (0..z.len())
        .find(|&i| z.by_index_raw(i).ok().and_then(|f| f.name().ok().map(|n| n == name)).unwrap_or(false))
        .ok_or_else(|| anyhow!("There is no item named '{name}' in the archive"))?;
    let mut f = z.by_index(index)?;
    if f.size() > MAX_ENTRY {
        bail!("{name} is too large");
    }
    let mut v = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut v)?;
    Ok(v)
}

pub fn open_f3d(source: &[u8]) -> Result<F3d> {
    let mut z = zip::ZipArchive::new(Cursor::new(source))?;
    let names: Vec<String> = z.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
    let mut metas: Vec<&String> = names.iter().filter(|n| n.ends_with("FusionDesignSegmentType1/MetaStream.dat")).collect();
    if metas.is_empty() {
        metas = names.iter().filter(|n| n.ends_with("Design1/MetaStream.dat")).collect();
    }
    if metas.is_empty() {
        return Err(F3dError("no Design segment in this file".into()).into());
    }
    metas.sort_by_key(|n| (!n.contains("[Active]"), (*n).clone()));
    let meta_name = metas[0].clone();
    let mut seg = parse_meta(&read_zip_entry(&mut z, &meta_name)?)?;
    seg.bulk = read_zip_entry(&mut z, &meta_name.replace("MetaStream.dat", "BulkStream.dat"))?;
    let redirections = if names.iter().any(|n| n == "RedirectionsStream.dat") {
        read_zip_entry(&mut z, "RedirectionsStream.dat")
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
    } else {
        None
    };
    Ok(F3d { design: seg, redirections })
}

// --------------------------------------------------------------------------- assembly structure

/// (min, max), mm.
pub type Box3 = ([f64; 3], [f64; 3]);

pub struct Body {
    /// The BREP blob's basename, "BREP.<uuid>.smbh".
    pub brep: String,
    pub selector: u64,
    /// The box Fusion cached for it, in the component's coordinates.
    pub bbox: Option<Box3>,
}

pub struct Component {
    pub name: String,
    pub space: u64,
    pub bodies: Vec<Body>,
}

impl Component {
    pub fn boxes(&self) -> Vec<Box3> {
        self.bodies.iter().filter_map(|b| b.bbox).collect()
    }
}

pub struct Placement {
    pub id: u64,
    /// The component placed (None: Python's -1, no target read).
    pub target: Option<u64>,
    /// The component it is placed in (None: the document root).
    pub parent: Option<u64>,
    pub matrix: Mat4,
    /// Cross-document link name (neutronRole) when it places another document.
    pub external: Option<String>,
}

pub struct Assembly {
    /// By component record id.
    pub components: IndexMap<u64, Component>,
    pub placements: Vec<Placement>,
    pub root: Option<u64>,
}

static UTF16: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u)(?:[\x20-\x7e]\x00){2,}").expect("utf16 pattern"));
static REF3: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u)\x01\x00\x01").expect("ref pattern"));
static PARENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s-u)\x01(.{8})\x00\x00").expect("parent pattern"));

fn utf16_strings(b: &[u8]) -> Vec<String> {
    UTF16.find_iter(b).map(|m| m.as_bytes().iter().step_by(2).map(|c| *c as char).collect()).collect()
}

fn component_name(rec: &[u8]) -> String {
    let strings = utf16_strings(&rec[..rec.len().min(1200)]);
    for (i, s) in strings.iter().enumerate() {
        if s == "Part Number" && i + 1 < strings.len() {
            return strings[i + 1].clone();
        }
    }
    strings.last().cloned().unwrap_or_else(|| "Componente".into())
}

fn rigid(v: &[f64; 16]) -> bool {
    if !v.iter().all(|x| x.is_finite()) {
        return false;
    }
    if [12, 13, 14].iter().any(|&k| v[k].abs() > 1e-9) || (v[15] - 1.0).abs() > 1e-9 {
        return false;
    }
    let rows = [&v[0..3], &v[4..7], &v[8..11]];
    if rows.iter().any(|r| (py_sum(r.iter().map(|x| x * x)) - 1.0).abs() > 1e-6) {
        return false;
    }
    (0..3).all(|a| (a + 1..3).all(|c| py_sum((0..3).map(|k| rows[a][k] * rows[c][k])).abs() < 1e-6))
}

fn doubles16(b: &[u8], at: usize) -> [f64; 16] {
    std::array::from_fn(|k| f64_at(b, at + 8 * k))
}

/// The matrix with its translation from centimetres to millimetres.
fn to_mm(v: [f64; 16]) -> Mat4 {
    std::array::from_fn(|k| v[k] * if k == 3 || k == 7 || k == 11 { 10.0 } else { 1.0 })
}

fn find_matrix(rec: &[u8]) -> Option<Mat4> {
    (0..rec.len().saturating_sub(127)).map(|i| doubles16(rec, i)).find(rigid).map(to_mm)
}

fn rigid_inverse(m: &Mat4) -> Mat4 {
    let r = |i: usize, j: usize| m[4 * i + j];
    let t = [m[3], m[7], m[11]];
    let rt = |i: usize, j: usize| r(j, i);
    let ti: Vec<f64> = (0..3).map(|i| -py_sum((0..3).map(|k| rt(i, k) * t[k]))).collect();
    [rt(0, 0), rt(0, 1), rt(0, 2), ti[0], rt(1, 0), rt(1, 1), rt(1, 2), ti[1], rt(2, 0), rt(2, 1), rt(2, 2), ti[2], 0.0, 0.0, 0.0, 1.0]
}

pub fn matmul4(a: &Mat4, b: &Mat4) -> Mat4 {
    std::array::from_fn(|i| {
        let (r, c) = (i / 4, i % 4);
        py_sum((0..4).map(|k| a[4 * r + k] * b[4 * k + c]))
    })
}

/// Occurrence transform object -> the world matrix Fusion shows it at.
///
/// A transform cache record names the occurrence's transform object, then holds one matrix per
/// context, each after a reference to its context: the occurrence itself (its own matrix), a
/// position, a joint... Fusion displays the joint's one; without a joint, the last.
fn cached_world(s: &Segment) -> Result<HashMap<u64, Mat4>> {
    let mut out = HashMap::new();
    for &e in s.of_type(TRANSFORM_CACHE) {
        let rec = s.record(e)?;
        let mut refs: Vec<(usize, u64)> = Vec::new();
        for m in REF3.find_iter(rec) {
            if m.start() + 11 <= rec.len() {
                let r = u64_at(rec, m.start() + 3);
                if s.index.contains_key(&r) {
                    refs.push((m.start(), r));
                }
            }
        }
        let Some(&(first_at, owner)) = refs.first() else { continue };
        let mut found: Vec<(u64, Mat4)> = Vec::new();
        let mut i = first_at + 11;
        while rec.len() >= 128 && i <= rec.len() - 128 {
            let v = doubles16(rec, i);
            if rigid(&v) {
                let context = refs.iter().filter(|(o, _)| *o < i).map(|(_, r)| *r).next_back().unwrap_or(owner);
                found.push((context, to_mm(v)));
                i += 128;
            } else {
                i += 1;
            }
        }
        let shown: Vec<&Mat4> = found.iter().filter(|(c, _)| *c != owner && s.guid_of(*c) == Some(JOINT)).map(|(_, m)| m).collect();
        let others: Vec<&Mat4> = found.iter().filter(|(c, _)| *c != owner).map(|(_, m)| m).collect();
        if let Some(m) = shown.last().or(others.last()) {
            out.insert(owner, **m);
        }
    }
    Ok(out)
}

/// Give joint-placed occurrences the local matrix that puts them where Fusion shows them.
fn solve_free(s: &Segment, placements: &mut [Placement], free: &[u64], root: Option<u64>) -> Result<()> {
    if free.is_empty() {
        return Ok(());
    }
    let cached = cached_world(s)?;
    let mut by_target: HashMap<u64, usize> = HashMap::new();
    for (i, p) in placements.iter().enumerate() {
        if let Some(t) = p.target {
            by_target.insert(t, i);
        }
    }
    fn world(placements: &[Placement], by_target: &HashMap<u64, usize>, root: Option<u64>, cid: Option<u64>, depth: usize) -> Mat4 {
        let Some(p) = cid.and_then(|c| by_target.get(&c)).map(|&i| &placements[i]) else { return IDENTITY4 };
        if cid == root || depth > 32 {
            return IDENTITY4;
        }
        matmul4(&world(placements, by_target, root, p.parent, depth + 1), &p.matrix)
    }
    for i in 0..placements.len() {
        if !free.contains(&placements[i].id) {
            continue;
        }
        let obj = placements[i].id + 3; // the occurrence's transform object follows its placement
        if let (Some(m), Some(OCCURRENCE_TRANSFORM)) = (cached.get(&obj), s.guid_of(obj)) {
            let w = world(placements, &by_target, root, placements[i].parent, 0);
            placements[i].matrix = matmul4(&rigid_inverse(&w), m);
        }
    }
    Ok(())
}

pub fn read_assembly(f: &F3d) -> Result<Assembly> {
    let s = &f.design;
    let mut spaces: Vec<u64> = s.types.iter().filter(|t| t.base.to_uppercase() == NAMING_SPACE_BASE).flat_map(|t| t.ids.iter().copied()).collect();
    spaces.sort_unstable();
    let mut components: IndexMap<u64, Component> = IndexMap::new();
    // A component record is the nearest record before its naming space that carries "Part Number".
    for &sp in &spaces {
        let (from, to) = (sp as i128 - 1, (sp as i128 - 6).max(0));
        let mut cand = from;
        while cand > to {
            let c = cand as u64;
            if s.index.contains_key(&c) && utf16_strings(&s.record(c)?[..s.record(c)?.len().min(1200)]).iter().any(|x| x == "Part Number") {
                components.insert(c, Component { name: component_name(s.record(c)?), space: sp, bodies: Vec::new() });
                break;
            }
            cand -= 1;
        }
    }
    let by_space: HashMap<u64, u64> = components.iter().map(|(id, c)| (c.space, *id)).collect();
    let mut roots: Vec<u64> = components.keys().copied().collect();
    roots.sort_unstable();
    let mut placements: Vec<Placement> = Vec::new();
    let mut free: Vec<u64> = Vec::new(); // placements that store no matrix: a joint places them
    let mut root = None;
    for &e in s.of_type(PLACEMENT) {
        let rec = s.record(e)?;
        let mut r = Reader { data: rec, pos: 0 };
        let parsed = (|| -> Result<Option<(Option<u64>, Option<String>)>> {
            r.ascii()?;
            r.u64()?;
            r.ascii()?;
            if r.u8()? != 1 {
                return Ok(None);
            }
            let count = r.u32()?;
            let (mut target, mut external) = (None, None);
            for _ in 0..count {
                if r.u8()? == 0 {
                    continue;
                }
                let tgt = r.u64()?;
                if r.u8()? == 0 {
                    if r.u8()? == 1 {
                        r.u32()?;
                    }
                } else {
                    r.u32()?;
                    r.utf16()?;
                    if r.u8()? == 0 {
                        r.ascii()?;
                        external = Some(r.utf16()?);
                        if r.u8()? == 1 {
                            r.utf16()?;
                            r.utf16()?;
                        }
                    }
                }
                if target.is_none() {
                    target = Some(tgt);
                }
                r.u32()?;
            }
            Ok(Some((target, external)))
        })();
        let (target, external) = match parsed {
            Ok(Some(x)) => x,
            Ok(None) => continue,
            Err(e) if e.is::<F3dError>() => continue,
            Err(e) => return Err(e),
        };
        let mut parent = None;
        for m in PARENT.captures_iter(rec) {
            let r = u64::from_le_bytes(m[1].try_into().unwrap_or([0; 8]));
            if let Some(&c) = by_space.get(&r) {
                parent = Some(c);
            }
        }
        let matrix = find_matrix(rec);
        if parent.is_none() && target.is_some_and(|t| components.contains_key(&t)) && external.is_none() {
            root = target; // the root instance
            continue;
        }
        if matrix.is_none() {
            free.push(e);
        }
        placements.push(Placement { id: e, target, parent, matrix: matrix.unwrap_or(IDENTITY4), external });
    }
    solve_free(s, &mut placements, &free, root)?;
    // bodies: each body map names a BREP blob and the entity suffix that puts it in a component cluster
    for &e in s.of_type(BODY_MAP) {
        let rec = s.record(e)?;
        let names: Vec<String> = utf16_strings(rec).into_iter().filter(|x| x.starts_with("BREP.")).collect();
        let Some(brep) = names.first() else { continue };
        for (selector, suffix) in body_pairs(rec) {
            let i = roots.partition_point(|r| *r <= suffix);
            if i == 0 {
                continue;
            }
            let bbox = body_box(s, suffix)?;
            if let Some(c) = components.get_mut(&roots[i - 1]) {
                c.bodies.push(Body { brep: brep.clone(), selector, bbox });
            }
        }
    }
    Ok(Assembly { components, placements, root })
}

/// (body selector, entity suffix) pairs: a u32 count after the header's zero run.
fn body_pairs(rec: &[u8]) -> Vec<(u64, u64)> {
    for start in 15..40 {
        if start + 4 > rec.len() {
            break;
        }
        let n = u32::from_le_bytes(rec[start..start + 4].try_into().unwrap_or([0; 4])) as usize;
        if !(1..=64).contains(&n) || start + 4 + 16 * n > rec.len() {
            continue;
        }
        if rec[15..start].iter().any(|b| *b != 0) {
            continue;
        }
        let pairs: Vec<(u64, u64)> = (0..n).map(|k| (u64_at(rec, start + 4 + 16 * k), u64_at(rec, start + 12 + 16 * k))).collect();
        if pairs.iter().all(|&(sel, suf)| sel < 1 << 20 && 0 < suf && suf < 1 << 32) {
            return pairs;
        }
    }
    Vec::new()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w == needle)
}

/// The body's cached box: records suffix+1..3 each hold the same marker byte 1 and six f64
/// (xmax, ymax, zmax, xmin, ymin, zmin, centimetres).
fn body_box(s: &Segment, suffix: u64) -> Result<Option<Box3>> {
    let mut recs = Vec::new();
    for k in 1..=3 {
        if s.index.contains_key(&(suffix + k)) {
            recs.push(s.record(suffix + k)?);
        }
    }
    if recs.len() < 2 {
        return Ok(None);
    }
    let first = recs[0];
    for i in 0..first.len().saturating_sub(48) {
        if first[i] != 1 {
            continue;
        }
        let frame = &first[i..(i + 49).min(first.len())];
        if !recs[1..].iter().all(|r| contains(r, frame)) {
            continue;
        }
        if frame.len() < 49 {
            continue;
        }
        let v: [f64; 6] = std::array::from_fn(|k| f64_at(frame, 1 + 8 * k));
        if !v.iter().all(|x| x.is_finite() && (*x == 0.0 || (1e-9 < x.abs() && x.abs() < 1e6))) {
            continue;
        }
        if (0..3).all(|k| v[k] >= v[k + 3]) && (0..3).any(|k| v[k] - v[k + 3] > 1e-7) {
            return Ok(Some(([v[3] * 10.0, v[4] * 10.0, v[5] * 10.0], [v[0] * 10.0, v[1] * 10.0, v[2] * 10.0])));
        }
    }
    Ok(None)
}

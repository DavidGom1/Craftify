"""Read a Fusion design (.f3d) without Fusion: its container and Design segment.

Follows the format description cadmpeg publishes (docs/formats/f3d.md, CC BY 4.0). An .f3d is
a ZIP; the Design asset folder holds FusionDesignSegmentType1/MetaStream.dat (a type table and
record index) and BulkStream.dat (the records).
"""

from __future__ import annotations

import io
import json
import struct
import zipfile
from dataclasses import dataclass, field
from pathlib import Path

MAX_ENTRY = 512 * 1024 * 1024


class F3dError(ValueError):
    pass


class Reader:
    def __init__(self, data: bytes, pos: int = 0):
        self.data = data
        self.pos = pos

    def need(self, n: int) -> None:
        if self.pos + n > len(self.data):
            raise F3dError(f"truncated at {self.pos} (+{n})")

    def u8(self) -> int:
        self.need(1)
        v = self.data[self.pos]
        self.pos += 1
        return v

    def u32(self) -> int:
        self.need(4)
        v = struct.unpack_from("<I", self.data, self.pos)[0]
        self.pos += 4
        return v

    def u64(self) -> int:
        self.need(8)
        v = struct.unpack_from("<Q", self.data, self.pos)[0]
        self.pos += 8
        return v

    def f64(self) -> float:
        self.need(8)
        v = struct.unpack_from("<d", self.data, self.pos)[0]
        self.pos += 8
        return v

    def ascii(self) -> str:
        n = self.u32()
        if n > 1 << 20:
            raise F3dError(f"string too long at {self.pos}")
        self.need(n)
        s = self.data[self.pos:self.pos + n].decode("latin-1")
        self.pos += n
        return s

    def utf16(self) -> str:
        n = self.u32()
        if n > 1 << 20:
            raise F3dError(f"string too long at {self.pos}")
        self.need(2 * n)
        s = self.data[self.pos:self.pos + 2 * n].decode("utf-16le", errors="replace")
        self.pos += 2 * n
        return s


@dataclass
class TypeEntry:
    guid: str
    base: str
    version: int
    addin: str
    ids: list[int]
    tag: int  # the dynamic class tag records of this type carry (256 + table index)


@dataclass
class Segment:
    name: str
    segment_id: int
    asset: str
    magic: int
    full_name: str
    addin: str
    kind: int
    types: list[TypeEntry]
    named: list[int]
    index: dict[int, int]  # entity id -> BulkStream offset
    secondary: dict[int, int]
    bulk: bytes = b""
    type_of: dict[int, TypeEntry] = field(default_factory=dict)

    def record(self, entity: int) -> bytes:
        """The bytes of one record: from its offset to the next record's."""
        start = self.index[entity]
        later = [o for o in self._offsets if o > start]
        return self.bulk[start:(later[0] if later else len(self.bulk))]

    @property
    def _offsets(self) -> list[int]:
        if not hasattr(self, "_sorted"):
            self._sorted = sorted(self.index.values())
        return self._sorted

    def of_type(self, guid: str) -> list[int]:
        for t in self.types:
            if t.guid.upper() == guid.upper():
                return t.ids
        return []


def parse_meta(data: bytes) -> Segment:
    r = Reader(data)
    name = r.ascii()
    seg_id = r.u32()
    asset = r.utf16()
    magic = r.u32()
    for _ in range(3 if magic == 1234 else 1):
        r.u32()
    full = r.ascii()
    addin = r.ascii()
    kind = r.u32()
    r.u32()
    types = []
    for i in range(r.u32()):
        guid = r.ascii()
        base = r.ascii()
        ver = r.u32()
        add = r.ascii()
        ids = [r.u64() for _ in range(r.u32())]
        types.append(TypeEntry(guid, base, ver, add, ids, 256 + i))
    named = [r.u64() for _ in range(r.u32())]
    index = {}
    for _ in range(r.u32()):
        e = r.u64()
        index[e] = r.u64()
    secondary = {}
    for _ in range(r.u32()):
        e = r.u64()
        secondary[e] = r.u64()
    seg = Segment(name, seg_id, asset, magic, full, addin, kind, types, named, index, secondary)
    for t in types:
        for e in t.ids:
            seg.type_of[e] = t
    return seg


@dataclass
class F3d:
    path: Path
    zip: zipfile.ZipFile
    asset: str  # e.g. "FusionAssetName[Active]"
    design: Segment
    redirections: dict | None

    def entry(self, name: str) -> bytes:
        info = self.zip.getinfo(name)
        if info.file_size > MAX_ENTRY:
            raise F3dError(f"{name} is too large")
        return self.zip.read(info)


def open_f3d(source: Path | bytes) -> F3d:
    z = zipfile.ZipFile(io.BytesIO(source) if isinstance(source, bytes) else source)
    names = z.namelist()
    metas = [n for n in names if n.endswith("FusionDesignSegmentType1/MetaStream.dat")]
    if not metas:
        metas = [n for n in names if n.endswith("Design1/MetaStream.dat")]
    if not metas:
        raise F3dError("no Design segment in this file")
    meta_name = sorted(metas, key=lambda n: ("[Active]" not in n, n))[0]
    folder = meta_name.rsplit("/", 2)[0]
    seg = parse_meta(z.read(meta_name))
    seg.bulk = z.read(meta_name.replace("MetaStream.dat", "BulkStream.dat"))
    redirections = None
    if "RedirectionsStream.dat" in names:
        try:
            redirections = json.loads(z.read("RedirectionsStream.dat").decode("utf-8"))
        except Exception:
            redirections = None
    return F3d(Path(source) if not isinstance(source, bytes) else Path("<memory>"), z, folder, seg, redirections)


# --------------------------------------------------------------------------- assembly structure

import bisect
import math
import re

PLACEMENT = "CE2913AA-CFE0-4F04-9102-24424ED3BCFA"  # occurrence placement (add-in Component)
NAMING_SPACE_BASE = "21F379C8-CAFD-4985-B461-767673A4C502"  # component naming space (base type)
BODY_MAP = "74CA4562-59F8-4C97-8AD1-F8297C21F9AA"  # BREP body map (add-in Body)
TRANSFORM_CACHE = "549FBB80-B890-473E-A5C0-415D3D9BF4E6"  # solved world transforms of an occurrence
OCCURRENCE_TRANSFORM = "66E375EA-40F1-4115-8D83-0F23DD30593E"  # an occurrence's transform object
JOINT = "8DFDD543-F823-43AA-BDAF-C638E023F0A0"  # the context Fusion shows a joint-driven occurrence in
IDENTITY4 = (1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0)


Box = tuple[tuple[float, float, float], tuple[float, float, float]]  # (min, max), mm


@dataclass
class Body:
    brep: str  # the BREP blob's basename, "BREP.<uuid>.smbh"
    selector: int
    box: Box | None  # the box Fusion cached for it, in the component's coordinates


@dataclass
class Component:
    id: int  # the component record's entity id (its cluster starts here)
    name: str
    space: int  # naming-space entity id
    bodies: list[Body] = field(default_factory=list)

    @property
    def boxes(self) -> list[Box]:
        return [b.box for b in self.bodies if b.box]


@dataclass
class Placement:
    id: int
    target: int  # component id placed
    parent: int | None  # component id it is placed in (None: the document root)
    matrix: tuple[float, ...]  # row-major 4x4, translation in mm
    external: str | None = None  # cross-document link name (neutronRole) when it places another document


@dataclass
class Assembly:
    components: dict[int, Component]
    placements: list[Placement]
    root: int | None


def _utf16_strings(b: bytes) -> list[str]:
    return [m.group(0).decode("utf-16le") for m in re.finditer(rb"(?:[\x20-\x7e]\x00){2,}", b)]


def _component_name(rec: bytes) -> str:
    strings = _utf16_strings(rec[:1200])
    for i, s in enumerate(strings):
        if s == "Part Number" and i + 1 < len(strings):
            return strings[i + 1]
    return strings[-1] if strings else "Componente"


def _rigid(v: tuple[float, ...]) -> bool:
    if not all(math.isfinite(x) for x in v):
        return False
    if any(abs(v[k]) > 1e-9 for k in (12, 13, 14)) or abs(v[15] - 1.0) > 1e-9:
        return False
    rows = (v[0:3], v[4:7], v[8:11])
    if any(abs(sum(x * x for x in r) - 1.0) > 1e-6 for r in rows):
        return False
    return all(abs(sum(rows[a][k] * rows[c][k] for k in range(3))) < 1e-6 for a in range(3) for c in range(a + 1, 3))


def _find_matrix(rec: bytes) -> tuple[float, ...] | None:
    for i in range(0, len(rec) - 127):
        v = struct.unpack_from("<16d", rec, i)
        if _rigid(v):
            return tuple(v[k] * (10.0 if k in (3, 7, 11) else 1.0) for k in range(16))
    return None


def _rigid_inverse(m: tuple[float, ...]) -> tuple[float, ...]:
    r = [[m[4 * i + j] for j in range(3)] for i in range(3)]
    t = (m[3], m[7], m[11])
    rt = [[r[j][i] for j in range(3)] for i in range(3)]
    ti = [-sum(rt[i][k] * t[k] for k in range(3)) for i in range(3)]
    return (rt[0][0], rt[0][1], rt[0][2], ti[0], rt[1][0], rt[1][1], rt[1][2], ti[1],
            rt[2][0], rt[2][1], rt[2][2], ti[2], 0.0, 0.0, 0.0, 1.0)


def _matmul4(a: tuple[float, ...], b: tuple[float, ...]) -> tuple[float, ...]:
    return tuple(sum(a[4 * r + k] * b[4 * k + c] for k in range(4)) for r in range(4) for c in range(4))


def _cached_world(s: Segment) -> dict[int, tuple[float, ...]]:
    """Occurrence transform object -> the world matrix Fusion shows it at.

    A transform cache record names the occurrence's transform object, then holds one matrix per
    context, each after a reference to its context: the occurrence itself (its own matrix), a
    position, a joint... Fusion displays the joint's one; without a joint, the last."""
    out = {}
    for e in s.of_type(TRANSFORM_CACHE):
        rec = s.record(e)
        refs = []
        for m in re.finditer(rb"\x01\x00\x01", rec):
            if m.start() + 11 <= len(rec):
                r = struct.unpack_from("<Q", rec, m.start() + 3)[0]
                if r in s.index:
                    refs.append((m.start(), r))
        if not refs:
            continue
        owner = refs[0][1]
        found = []
        i = refs[0][0] + 11
        while i <= len(rec) - 128:
            v = struct.unpack_from("<16d", rec, i)
            if _rigid(v):
                context = [r for o, r in refs if o < i][-1]
                found.append((context, tuple(v[k] * (10.0 if k in (3, 7, 11) else 1.0) for k in range(16))))
                i += 128
            else:
                i += 1
        shown = [m for c, m in found if c != owner and s.type_of.get(c) is not None and s.type_of[c].guid == JOINT]
        others = [m for c, m in found if c != owner]
        if shown or others:
            out[owner] = (shown or others)[-1]
    return out


def _solve_free(s: Segment, placements: list[Placement], free: list[int], root: int | None) -> None:
    """Give joint-placed occurrences the local matrix that puts them where Fusion shows them."""
    if not free:
        return
    cached = _cached_world(s)
    by_target = {p.target: p for p in placements}

    def world(cid: int | None, depth: int = 0) -> tuple[float, ...]:
        p = by_target.get(cid)
        if p is None or cid == root or depth > 32:
            return IDENTITY4
        return _matmul4(world(p.parent, depth + 1), p.matrix)

    for p in placements:
        if p.id in free:
            obj = p.id + 3  # the occurrence's transform object follows its placement
            t = s.type_of.get(obj)
            if obj in cached and t is not None and t.guid == OCCURRENCE_TRANSFORM:
                p.matrix = _matmul4(_rigid_inverse(world(p.parent)), cached[obj])


def read_assembly(f: F3d) -> Assembly:
    s = f.design
    spaces = sorted(e for t in s.types if t.base.upper() == NAMING_SPACE_BASE for e in t.ids)
    comp_type = None
    components: dict[int, Component] = {}
    # A component record is the nearest record before its naming space that carries "Part Number".
    for sp in spaces:
        for cand in range(sp - 1, max(sp - 6, 0), -1):
            if cand in s.index and "Part Number" in _utf16_strings(s.record(cand)[:1200]):
                components[cand] = Component(cand, _component_name(s.record(cand)), sp)
                comp_type = comp_type or s.type_of.get(cand)
                break
    by_space = {c.space: c.id for c in components.values()}
    roots = sorted(components)
    placements: list[Placement] = []
    free: list[int] = []  # placements that store no matrix: a joint places them
    root = None
    for e in s.of_type(PLACEMENT):
        rec = s.record(e)
        r = Reader(rec)
        try:
            r.ascii(); r.u64(); r.ascii()
            if r.u8() != 1:
                continue
            count = r.u32()
            target = None
            external = None
            for _ in range(count):
                present = r.u8()
                if not present:
                    continue
                tgt = r.u64()
                cross = r.u8()
                if cross == 0:
                    if r.u8() == 1:
                        r.u32()
                else:
                    r.u32(); r.utf16()
                    if r.u8() == 0:
                        r.ascii(); external = r.utf16()
                        if r.u8() == 1:
                            r.utf16(); r.utf16()
                if target is None:
                    target = tgt
                r.u32()
        except F3dError:
            continue
        parent = None
        for m in re.finditer(rb"\x01(.{8})\x00\x00", rec, re.S):
            ref = struct.unpack("<Q", m.group(1))[0]
            if ref in by_space:
                parent = by_space[ref]
        matrix = _find_matrix(rec)
        if parent is None and target in components and external is None:
            root = target  # the root instance
            continue
        if matrix is None:
            free.append(e)
        placements.append(Placement(e, target if target is not None else -1, parent, matrix or IDENTITY4, external))
    _solve_free(s, placements, free, root)
    # bodies: each body map names a BREP blob and the entity suffix that puts it in a component cluster
    for e in s.of_type(BODY_MAP):
        rec = s.record(e)
        m = re.search(rb"((?:[\x20-\x7e]\x00)+)", rec[40:])
        names = [x for x in _utf16_strings(rec) if x.startswith("BREP.")]
        if not names:
            continue
        pairs = _body_pairs(rec)
        for selector, suffix in pairs:
            i = bisect.bisect_right(roots, suffix) - 1
            if i < 0:
                continue
            components[roots[i]].bodies.append(Body(names[0], selector, _body_box(s, suffix)))
    return Assembly(components, placements, root)


def _body_pairs(rec: bytes) -> list[tuple[int, int]]:
    """(body selector, entity suffix) pairs: a u32 count after the header's zero run."""
    for start in range(15, 40):
        if start + 4 > len(rec):
            break
        n = struct.unpack_from("<I", rec, start)[0]
        if not 1 <= n <= 64 or start + 4 + 16 * n > len(rec):
            continue
        if any(rec[k] for k in range(15, start)):
            continue
        pairs = [struct.unpack_from("<QQ", rec, start + 4 + 16 * k) for k in range(n)]
        if all(sel < 1 << 20 and 0 < suf < 1 << 32 for sel, suf in pairs):
            return pairs
    return []


def _body_box(s: Segment, suffix: int):
    """The body's cached box: records suffix+1..3 each hold the same marker byte 1 and six f64
    (xmax, ymax, zmax, xmin, ymin, zmin, centimetres)."""
    recs = [s.record(suffix + k) for k in (1, 2, 3) if suffix + k in s.index]
    if len(recs) < 2:
        return None
    first = recs[0]
    for i in range(len(first) - 48):
        if first[i] != 1:
            continue
        frame = first[i:i + 49]
        if not all(frame in r for r in recs[1:]):
            continue
        v = struct.unpack_from("<6d", frame, 1)
        if not all(math.isfinite(x) and (x == 0 or 1e-9 < abs(x) < 1e6) for x in v):
            continue
        if all(v[k] >= v[k + 3] for k in range(3)) and any(v[k] - v[k + 3] > 1e-7 for k in range(3)):
            return (v[3] * 10, v[4] * 10, v[5] * 10), (v[0] * 10, v[1] * 10, v[2] * 10)
    return None

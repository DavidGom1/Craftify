"""Tidy a STEP file written by cadmpeg the way Fusion's own STEP export would write it.

SolidCraft reads some of what cadmpeg writes wrong (faces trimmed to half their area, gaps
between faces), though it is valid STEP. Fusion's own export differs in three ways, which this
module copies:

* Seams. A face that goes all the way round a cylinder or cone comes from cadmpeg the way the
  source CAD keeps it: two loops, one at each end (or one loop and the cone's apex), and no
  seam. Fusion cuts it open along a ruling line, so the face has a single loop that runs along
  the seam once in each direction. The seam runs through a vertex of one loop; where the other
  loop has no vertex at that angle, the edge that crosses it is split there (both faces that use
  the edge get the two halves).
* Trimmed curves. cadmpeg gives an edge the whole cached B-spline of its curve, which can run
  well past the edge's ends; Fusion writes only the stretch between the edge's vertices.
* Tolerance. The vertices of the cached curves sit a few micrometres off them; the declared
  model tolerance must allow for that.
"""

from __future__ import annotations

import math
import re
from dataclasses import dataclass

from .step_fix import Entity, Step, Vec, _add, _dot, _mul, _norm, _ref, _split_args, _sub, _unit

_ROUND = ("CYLINDRICAL_SURFACE", "CONICAL_SURFACE")
_COMPLEX = re.compile(r"#(\d+)\s*=\s*\((.*)\)\s*;\s*$", re.S)
SAME_POINT = 1e-6  # mm: a split this close to a vertex uses the vertex


def _fmt(a) -> str:
    return "(" + ",".join(_fmt(x) for x in a) + ")" if isinstance(a, list) else a


def _parts(text: str) -> dict[str, list]:
    """The named parts of a complex entity: 'A(..) B(..)' -> {A: args, B: args}."""
    out: dict[str, list] = {}
    i = 0
    while i < len(text):
        m = re.compile(r"\s*([A-Z_0-9]+)\s*\(").match(text, i)
        if not m:
            break
        depth, j = 1, m.end()
        while j < len(text) and depth:
            if text[j] == "'":
                j = text.index("'", j + 1)
            elif text[j] == "(":
                depth += 1
            elif text[j] == ")":
                depth -= 1
            j += 1
        out[m.group(1)] = _split_args(text[m.end():j - 1])
        i = j
    return out


class _Edit(Step):
    def __init__(self, text: str):
        super().__init__(text)
        self.complex: dict[int, dict[str, list]] = {}
        for k, st in enumerate(self.statements):
            m = _COMPLEX.search(st.strip())
            if m and int(m.group(1)) not in self.ent:
                self.complex[int(m.group(1))] = _parts(m.group(2))
        self.next_id = max(list(self.ent) + list(self.complex) + [0]) + 1
        self.new: list[int] = []
        self.raw: dict[int, str] = {}
        self.users: dict[int, set[int]] = {}
        for eid, e in self.ent.items():
            for r in self._refs(e.args):
                self.users.setdefault(r, set()).add(eid)

    def _refs(self, args) -> list[int]:
        out = []
        for a in args:
            if isinstance(a, list):
                out += self._refs(a)
            elif a.startswith("#"):
                out.append(_ref(a))
        return out

    def add(self, type_: str, args: list) -> int:
        eid = self.next_id
        self.next_id += 1
        self.ent[eid] = Entity(type_, args, -1)
        self.new.append(eid)
        for r in self._refs(args):
            self.users.setdefault(r, set()).add(eid)
        return eid

    def set(self, eid: int, args: list) -> None:
        e = self.ent[eid]
        for r in self._refs(e.args):
            self.users.get(r, set()).discard(eid)
        e.args = args
        for r in self._refs(args):
            self.users.setdefault(r, set()).add(eid)
        if e.line >= 0:
            lead = re.match(r"\s*", self.statements[e.line]).group(0)
            self.statements[e.line] = f"{lead}#{eid} = {e.type}{_fmt(args)};"

    def drop(self, eid: int) -> None:
        """Remove an entity nothing refers to any more."""
        if self.users.get(eid):
            return
        e = self.ent.pop(eid)
        for r in self._refs(e.args):
            self.users.get(r, set()).discard(eid)
        if e.line >= 0:
            self.statements[e.line] = ""

    def add_complex(self, body: str) -> int:
        """A complex entity, written as given: '( A(..) B(..) )'."""
        eid = self.next_id
        self.next_id += 1
        self.raw[eid] = body
        self.new.append(eid)
        for r in re.findall(r"#(\d+)", body):
            self.users.setdefault(int(r), set()).add(eid)
        return eid

    def text(self) -> str:
        new = "".join(f"\n#{i} = {self.raw[i]};" if i in self.raw else
                      f"\n#{i} = {self.ent[i].type}{_fmt(self.ent[i].args)};"
                      for i in self.new if i in self.ent or i in self.raw)
        end = max(k for k, s in enumerate(self.statements) if s.strip() == "ENDSEC;")
        return "".join(self.statements[:end]) + new + "".join(self.statements[end:])

    def point_entity(self, p: Vec) -> int:
        return self.add("CARTESIAN_POINT", ["''", [repr(float(p[0])), repr(float(p[1])), repr(float(p[2]))]])


# --------------------------------------------------------------------------- curves

def _wrap(a: float) -> float:
    return (a + math.pi) % (2 * math.pi) - math.pi


class _Curve:
    """A curve that can be evaluated at a parameter: lines, circles, ellipses and B-splines."""

    def __init__(self, st: _Edit, cid: int):
        e = st.ent.get(cid)
        while e is not None and e.type in ("SURFACE_CURVE", "SEAM_CURVE"):
            cid = _ref(e.args[1])
            e = st.ent.get(cid)
        self.kind = None
        if e is not None and e.type == "LINE":
            self.kind = "line"
            self.o = st.point(_ref(e.args[1]))
            vec = st.ent[_ref(e.args[2])]
            self.d = _mul(st.direction(_ref(vec.args[1])), float(vec.args[2]))
        elif e is not None and e.type in ("CIRCLE", "ELLIPSE"):
            self.kind = "conic"
            self.o, self.x, self.y, _z = st.placement(_ref(e.args[1]))
            self.a = float(e.args[2])
            self.b = self.a if e.type == "CIRCLE" else float(e.args[3])
        else:
            parts = st.complex.get(cid)
            if e is not None and e.type == "B_SPLINE_CURVE_WITH_KNOTS":
                deg, pts, mults, knots, weights = e.args[1], e.args[2], e.args[6], e.args[7], None
            elif parts and "B_SPLINE_CURVE" in parts and "B_SPLINE_CURVE_WITH_KNOTS" in parts:
                deg, pts = parts["B_SPLINE_CURVE"][0], parts["B_SPLINE_CURVE"][1]
                mults, knots = parts["B_SPLINE_CURVE_WITH_KNOTS"][0], parts["B_SPLINE_CURVE_WITH_KNOTS"][1]
                weights = parts.get("RATIONAL_B_SPLINE_CURVE", [None])[0]
            else:
                return
            self.kind = "spline"
            self.p = int(deg)
            self.ctrl = [st.point(_ref(r)) for r in pts]
            self.w = [float(v) for v in weights] if weights else [1.0] * len(self.ctrl)
            self.u = [float(k) for k, m in zip(knots, mults) for _ in range(int(m))]

    def at(self, t: float) -> Vec:
        if self.kind == "line":
            return _add(self.o, _mul(self.d, t))
        if self.kind == "conic":
            return _add(self.o, _add(_mul(self.x, self.a * math.cos(t)), _mul(self.y, self.b * math.sin(t))))
        p, u, n = self.p, self.u, len(self.ctrl) - 1
        t = min(max(t, u[p]), u[n + 1])
        k = p
        while k < n and t >= u[k + 1]:
            k += 1
        d = [(*_mul(self.ctrl[j], self.w[j]), self.w[j]) for j in range(k - p, k + 1)]
        for r in range(1, p + 1):
            for j in range(p, r - 1, -1):
                lo, hi = u[j + k - p], u[j + 1 + k - r]
                a = 0.0 if hi == lo else (t - lo) / (hi - lo)
                d[j] = tuple((1 - a) * d[j - 1][m] + a * d[j][m] for m in range(4))
        x = d[p]
        return (x[0] / x[3], x[1] / x[3], x[2] / x[3])

    def domain(self) -> tuple[float, float]:
        if self.kind == "spline":
            return self.u[self.p], self.u[len(self.ctrl)]
        if self.kind == "conic":
            return 0.0, 2 * math.pi
        return 0.0, 1.0

    def param(self, q: Vec) -> float:
        """The parameter of the curve's point nearest q."""
        if self.kind == "line":
            dd = _dot(self.d, self.d)
            return _dot(_sub(q, self.o), self.d) / dd if dd else 0.0
        if self.kind == "conic":
            d = _sub(q, self.o)
            return math.atan2(_dot(d, self.y) / self.b, _dot(d, self.x) / self.a)
        if not hasattr(self, "table"):
            lo, hi = self.domain()
            n = max(64, 16 * len(self.ctrl))
            ts = [lo + (hi - lo) * i / n for i in range(n + 1)]
            self.table = (ts, [self.at(t) for t in ts])
        ts, pts = self.table
        i = min(range(len(ts)), key=lambda i: _norm(_sub(pts[i], q)))
        a, b = ts[max(i - 1, 0)], ts[min(i + 1, len(ts) - 1)]
        for _ in range(60):  # golden section
            m1, m2 = b - (b - a) * 0.618, a + (b - a) * 0.618
            if _norm(_sub(self.at(m1), q)) < _norm(_sub(self.at(m2), q)):
                b = m2
            else:
                a = m1
        return (a + b) / 2


@dataclass
class _Step:
    """One oriented edge of a loop, as travelled."""
    edge: int
    forward: bool  # travelled from its start vertex to its end vertex
    start: int  # vertex ids, in travel order
    end: int
    params: list[float]  # increasing along the curve or not, in travel order
    points: list[Vec]


def _travel(st: _Edit, ec_id: int, forward: bool, curves: dict[int, _Curve]) -> _Step | None:
    ec = st.ent[ec_id]  # EDGE_CURVE('', v1, v2, curve, same_sense)
    v1, v2 = _ref(ec.args[1]), _ref(ec.args[2])
    cid = _ref(ec.args[3])
    c = curves.get(cid) or curves.setdefault(cid, _Curve(st, cid))
    if c.kind is None:
        return None
    p1, p2 = st.vertex(v1), st.vertex(v2)
    t1, t2 = c.param(p1), c.param(p2)
    along = ec.args[4] == ".T."  # v1 -> v2 runs the curve's own way
    if c.kind == "conic":
        span = (t2 - t1) % (2 * math.pi) if along else -((t1 - t2) % (2 * math.pi))
        if v1 == v2 or abs(span) < 1e-12:
            span = 2 * math.pi if along else -2 * math.pi
        t2 = t1 + span
    elif v1 == v2:
        lo, hi = c.domain()
        t1, t2 = (lo, hi) if along else (hi, lo)
    n = 2 if c.kind == "line" else 24
    ts = [t1 + (t2 - t1) * i / n for i in range(n + 1)]
    pts = [c.at(t) for t in ts]
    pts[0], pts[-1] = p1, p2
    if not forward:
        return _Step(ec_id, False, v2, v1, ts[::-1], pts[::-1])
    return _Step(ec_id, True, v1, v2, ts, pts)


@dataclass
class _Loop:
    bound: int
    steps: list[_Step]  # empty for a vertex loop
    pole: int | None = None  # the vertex of a vertex loop


def _loops(st: _Edit, face: Entity, curves: dict[int, _Curve]) -> list[_Loop] | None:
    out = []
    for b in face.args[1]:
        bound = st.ent[_ref(b)]
        loop = st.ent[_ref(bound.args[1])]
        same = bound.args[2] == ".T."
        if loop.type == "VERTEX_LOOP":
            out.append(_Loop(_ref(b), [], _ref(loop.args[1])))
            continue
        steps = []
        for oe_ref in loop.args[1]:
            oe = st.ent[_ref(oe_ref)]
            s = _travel(st, _ref(oe.args[3]), (oe.args[4] == ".T.") == same, curves)
            if s is None:
                return None
            steps.append(s)
        out.append(_Loop(_ref(b), steps if same else steps[::-1]))
    return out


# --------------------------------------------------------------------------- seams

class _Axis:
    def __init__(self, st: _Edit, surf: Entity):
        self.o, self.x, self.y, self.z = st.placement(_ref(surf.args[1]))

    def angle(self, p: Vec) -> float:
        d = _sub(p, self.o)
        return math.atan2(_dot(d, self.y), _dot(d, self.x))

    def radius(self, p: Vec) -> float:
        d = _sub(p, self.o)
        return _norm(_sub(d, _mul(self.z, _dot(d, self.z))))

    def winding(self, loop: _Loop) -> float:
        total, last = 0.0, None
        for s in loop.steps:
            for p in s.points:
                a = self.angle(p)
                if last is not None:
                    total += _wrap(a - last)
                last = a
        return total / (2 * math.pi)


def _crossing(st: _Edit, ax: _Axis, loop: _Loop, theta: float, curves: dict[int, _Curve]):
    """Where the loop passes the angle theta: (step index, vertex id) or (step index, parameter, point)."""
    for k, s in enumerate(loop.steps):
        if ax.radius(st.vertex(s.start)) * abs(_wrap(ax.angle(st.vertex(s.start)) - theta)) < SAME_POINT * 100:
            return k, s.start, None
    for k, s in enumerate(loop.steps):
        c = curves[_ref(st.ent[s.edge].args[3])] if _ref(st.ent[s.edge].args[3]) in curves else None
        for i in range(len(s.points) - 1):
            a, b = _wrap(ax.angle(s.points[i]) - theta), _wrap(ax.angle(s.points[i + 1]) - theta)
            if abs(b) < 1e-12 and i + 1 < len(s.points) - 1:
                return k, None, (s.params[i + 1], c.at(s.params[i + 1]))
            if a == 0 or (a < 0) == (b < 0) or abs(a - b) > math.pi:
                continue
            lo, hi = s.params[i], s.params[i + 1]
            for _ in range(80):
                mid = (lo + hi) / 2
                m = _wrap(ax.angle(c.at(mid)) - theta)
                if (m < 0) == (a < 0):
                    lo = mid
                else:
                    hi = mid
            t = (lo + hi) / 2
            return k, None, (t, c.at(t))
    return None


def _split(st: _Edit, ec_id: int, p: Vec) -> int:
    """Split an edge at p (a point of its curve); every loop that uses it gets both halves."""
    ec = st.ent[ec_id]
    vp = st.add("VERTEX_POINT", ["''", f"#{st.point_entity(p)}"])
    first = st.add("EDGE_CURVE", ["''", ec.args[1], f"#{vp}", ec.args[3], ec.args[4]])
    second = st.add("EDGE_CURVE", ["''", f"#{vp}", ec.args[2], ec.args[3], ec.args[4]])
    for oe_id in list(st.users.get(ec_id, ())):
        oe = st.ent[oe_id]
        if oe.type != "ORIENTED_EDGE":
            continue
        if oe.args[4] == ".T.":
            pair = [st.add("ORIENTED_EDGE", ["''", "*", "*", f"#{first}", ".T."]),
                    st.add("ORIENTED_EDGE", ["''", "*", "*", f"#{second}", ".T."])]
        else:
            pair = [st.add("ORIENTED_EDGE", ["''", "*", "*", f"#{second}", ".F."]),
                    st.add("ORIENTED_EDGE", ["''", "*", "*", f"#{first}", ".F."])]
        for loop_id in list(st.users.get(oe_id, ())):
            loop = st.ent[loop_id]
            items = []
            for r in loop.args[1]:
                items += [f"#{i}" for i in pair] if r == f"#{oe_id}" else [r]
            st.set(loop_id, [loop.args[0], items])
        st.drop(oe_id)
    st.drop(ec_id)
    return vp


def _seam(st: _Edit, va: int, vb: int) -> int:
    pa, pb = st.vertex(va), st.vertex(vb)
    d = _sub(pb, pa)
    u = _unit(d) or (0.0, 0.0, 1.0)
    direction = st.add("DIRECTION", ["''", [repr(u[0]), repr(u[1]), repr(u[2])]])
    vector = st.add("VECTOR", ["''", f"#{direction}", repr(_norm(d))])
    line = st.add("LINE", ["''", f"#{st.point_entity(pa)}", f"#{vector}"])
    return st.add("EDGE_CURVE", ["''", f"#{va}", f"#{vb}", f"#{line}", ".T."])


def _rotated(steps: list[_Step], v: int) -> list[_Step]:
    k = next(i for i, s in enumerate(steps) if s.start == v)
    return steps[k:] + steps[:k]


def _cut_open(st: _Edit, fid: int, curves: dict[int, _Curve]) -> bool:
    face = st.ent[fid]
    surf = st.ent.get(_ref(face.args[2]))
    if surf is None or surf.type not in _ROUND:
        return False
    loops = _loops(st, face, curves)
    if loops is None:
        return False
    ax = _Axis(st, surf)
    round_ = [lp for lp in loops if lp.steps and abs(ax.winding(lp)) > 0.5]
    poles = [lp for lp in loops if lp.pole is not None]
    if len(round_) == 2:
        a, b = round_
    elif len(round_) == 1 and len(poles) == 1 and surf.type == "CONICAL_SURFACE":
        a, b = round_[0], poles[0]
    else:
        return False

    # Where Fusion puts it: half a turn from the surface's reference direction (SolidCraft
    # trims these faces right only with the seam there).
    ends = []
    for lp in (a, b):
        if lp.pole is not None:
            ends.append(lp.pole)
            continue
        hit = _crossing(st, ax, lp, math.pi, curves)
        if hit is None:
            return False
        k, v, cut = hit
        if v is None:
            v = _split(st, lp.steps[k].edge, cut[1])
        ends.append(v)
    loops = {lp.bound: lp for lp in _loops(st, st.ent[fid], curves) or []}  # again, with any split edges
    if a.bound not in loops or b.bound not in loops:
        return False
    a, b = loops[a.bound], loops[b.bound]
    face = st.ent[fid]
    va, vb = ends

    seam = _seam(st, va, vb)
    items = []
    for s in _rotated(a.steps, va):
        items.append(st.add("ORIENTED_EDGE", ["''", "*", "*", f"#{s.edge}", ".T." if s.forward else ".F."]))
    items.append(st.add("ORIENTED_EDGE", ["''", "*", "*", f"#{seam}", ".T."]))
    if b.steps:
        for s in _rotated(b.steps, vb):
            items.append(st.add("ORIENTED_EDGE", ["''", "*", "*", f"#{s.edge}", ".T." if s.forward else ".F."]))
    items.append(st.add("ORIENTED_EDGE", ["''", "*", "*", f"#{seam}", ".F."]))
    loop = st.add("EDGE_LOOP", ["''", [f"#{i}" for i in items]])
    bound = st.add("FACE_OUTER_BOUND", ["''", f"#{loop}", ".T."])
    old = {f"#{a.bound}", f"#{b.bound}"}
    st.set(fid, [face.args[0], [r for r in face.args[1] if r not in old] + [f"#{bound}"], face.args[2], face.args[3]])
    for lp in (a, b):
        b_ent = st.ent[lp.bound]
        loop_id = _ref(b_ent.args[1])
        st.drop(lp.bound)
        oes = [_ref(r) for r in st.ent[loop_id].args[1]] if st.ent[loop_id].type == "EDGE_LOOP" else []
        st.drop(loop_id)
        for oe in oes:
            st.drop(oe)
    return True


# --------------------------------------------------------------------------- trimmed curves

def _insert(p: int, u: list[float], pw: list[tuple], t: float) -> tuple[list[float], list[tuple]]:
    """Insert the knot t once (Boehm); pw are homogeneous control points."""
    n = len(pw) - 1
    k = p
    while k < n and t >= u[k + 1]:
        k += 1
    q = []
    for i in range(n + 2):
        if i <= k - p:
            q.append(pw[i])
        elif i > k:
            q.append(pw[i - 1])
        else:
            a = (t - u[i]) / (u[i + p] - u[i])
            q.append(tuple((1 - a) * pw[i - 1][m] + a * pw[i][m] for m in range(4)))
    return u[:k + 1] + [t] + u[k + 1:], q


def _piece(p: int, u: list[float], pw: list[tuple], a: float, b: float) -> tuple[list[float], list[tuple]]:
    """The knots and control points of the stretch [a, b] of a B-spline."""
    eps = 1e-12 * max(1.0, abs(u[-1] - u[0]))
    ends = []
    for t in (a, b):
        same = [x for x in u if abs(x - t) <= eps]
        t = same[0] if same else t
        for _ in range(p - len(same)):
            u, pw = _insert(p, u, pw, t)
        ends.append(t)
    a, b = ends
    la = max(i for i, x in enumerate(u) if x == a)
    fb = min(i for i, x in enumerate(u) if x == b)
    return [a] * (p + 1) + u[la + 1:fb] + [b] * (p + 1), pw[la - p:fb]


def _trim(st: _Edit, ec_id: int, curves: dict[int, _Curve]) -> bool:
    ec = st.ent[ec_id]
    v1, v2 = _ref(ec.args[1]), _ref(ec.args[2])
    cid = _ref(ec.args[3])
    c = curves.get(cid) or curves.setdefault(cid, _Curve(st, cid))
    if v1 == v2 or c.kind != "spline":
        return False
    p1, p2 = st.vertex(v1), st.vertex(v2)
    t1, t2 = c.param(p1), c.param(p2)
    along = ec.args[4] == ".T."
    lo, hi = c.domain()
    span = hi - lo
    if (t1 > t2) == along or abs(t2 - t1) < 1e-9 * span:
        return False  # runs through the closing point of a closed curve, or is a point
    a, b = min(t1, t2), max(t1, t2)
    if a - lo < 1e-9 * span and hi - b < 1e-9 * span:
        return False  # already the whole curve
    knots, ctrl = _piece(c.p, list(c.u), [(*_mul(q, w), w) for q, w in zip(c.ctrl, c.w)], a, b)
    first, last = (p1, p2) if along else (p2, p1)
    ctrl[0] = (*_mul(first, ctrl[0][3]), ctrl[0][3])  # the ends exactly on the vertices
    ctrl[-1] = (*_mul(last, ctrl[-1][3]), ctrl[-1][3])
    pts = [f"#{st.point_entity((q[0] / q[3], q[1] / q[3], q[2] / q[3]))}" for q in ctrl]
    mults: list[int] = []
    values: list[float] = []
    for k in knots:
        if values and k == values[-1]:
            mults[-1] += 1
        else:
            values.append(k)
            mults.append(1)
    m = [str(x) for x in mults]
    v = [repr(x) for x in values]
    weights = [q[3] for q in ctrl]
    if all(abs(w - 1.0) < 1e-12 for w in weights):
        new = st.add("B_SPLINE_CURVE_WITH_KNOTS", ["''", str(c.p), pts, ".UNSPECIFIED.", ".F.", ".F.", m, v,
                                                   ".UNSPECIFIED."])
    else:
        new = st.add_complex(f"( BOUNDED_CURVE() B_SPLINE_CURVE({c.p},{_fmt(pts)},.UNSPECIFIED.,.F.,.F.) "
                             f"B_SPLINE_CURVE_WITH_KNOTS({_fmt(m)},{_fmt(v)},.UNSPECIFIED.) CURVE() "
                             f"GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE({_fmt([repr(w) for w in weights])}) "
                             f"REPRESENTATION_ITEM('') )")
    st.set(ec_id, [ec.args[0], ec.args[1], ec.args[2], f"#{new}", ec.args[4]])
    old = st.ent.get(cid)
    if old is not None and old.type in ("SURFACE_CURVE", "SEAM_CURVE") and not st.users.get(cid):
        pcurves = [_ref(r) for r in old.args[2]] if isinstance(old.args[2], list) else []
        st.drop(cid)
        for pc in pcurves:
            rep = _ref(st.ent[pc].args[2]) if pc in st.ent else None
            st.drop(pc)
            if rep is not None and rep in st.ent:
                st.drop(rep)
    return True


# --------------------------------------------------------------------------- outer shells

def _shell_size(st: _Edit, shell: int, curves: dict[int, _Curve]) -> float:
    """The diagonal of the box round a shell's edges (points along them, not only vertices: a
    circle has one vertex)."""
    pts: list[Vec] = []
    for f in st.ent[shell].args[1]:
        for b in st.ent[_ref(f)].args[1]:
            loop = st.ent[_ref(st.ent[_ref(b)].args[1])]
            if loop.type == "VERTEX_LOOP":
                pts.append(st.vertex(_ref(loop.args[1])))
                continue
            for oe in loop.args[1]:
                s = _travel(st, _ref(st.ent[_ref(oe)].args[3]), True, curves)
                if s is not None:
                    pts += s.points
    if not pts:
        return 0.0
    return math.dist([min(p[k] for p in pts) for k in range(3)], [max(p[k] for p in pts) for k in range(3)])


def _outer_shell(st: _Edit, brep: int, curves: dict[int, _Curve]) -> bool:
    """A solid with voids must have its largest shell as the outer one (cadmpeg may list a void)."""
    e = st.ent[brep]  # BREP_WITH_VOIDS('', outer, (oriented shells))
    outer = _ref(e.args[1])
    voids = [_ref(r) for r in e.args[2]]
    best = max(voids, key=lambda v: _shell_size(st, _ref(st.ent[v].args[2]), curves))
    inner = _ref(st.ent[best].args[2])
    if _shell_size(st, inner, curves) <= _shell_size(st, outer, curves):
        return False
    o = st.ent[best]  # ORIENTED_CLOSED_SHELL('', *, shell, .F.)
    st.set(best, [o.args[0], o.args[1], f"#{outer}", o.args[3]])
    st.set(brep, [e.args[0], f"#{inner}", e.args[2]])
    return True


def clean(text: str, tolerance_mm: float = 1e-3) -> tuple[str, dict[str, int]]:
    """The STEP text as Fusion's export would write it, and how many faces and edges changed."""
    st = _Edit(text)
    curves: dict[int, _Curve] = {}
    done = {"outer shells": 0, "seams": 0, "trimmed": 0}
    for brep in [i for i, e in st.ent.items() if e.type == "BREP_WITH_VOIDS"]:
        try:
            done["outer shells"] += _outer_shell(st, brep, curves)
        except (KeyError, ValueError, IndexError):
            continue
    for fid in [i for i, e in st.ent.items() if e.type == "ADVANCED_FACE"]:
        try:
            done["seams"] += _cut_open(st, fid, curves)
        except (KeyError, ValueError, ZeroDivisionError, StopIteration, IndexError):
            continue
    for ec_id in [i for i, e in st.ent.items() if e.type == "EDGE_CURVE"]:
        try:
            done["trimmed"] += _trim(st, ec_id, curves)
        except (KeyError, ValueError, ZeroDivisionError, IndexError):
            continue
    out = st.text() if any(done.values()) else text
    return set_tolerance(out, tolerance_mm), done


def set_tolerance(text: str, mm: float) -> str:
    """Declare the model's distance tolerance (readers check vertices on curves against it)."""
    return re.sub(r"(UNCERTAINTY_MEASURE_WITH_UNIT\s*\(\s*LENGTH_MEASURE\s*\()[^)]*\)",
                  lambda m: f"{m.group(1)}{mm!r})", text, count=1)

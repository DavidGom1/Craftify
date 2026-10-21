"""Repair face orientation flags (ADVANCED_FACE same_sense) in a STEP file.

Some converters write every face with the same ``same_sense`` flag. Where the loops can be
trusted they still carry the true orientation: ISO 10303-42 keeps a face on the left of its
bounding loops when seen from the face's outward normal.

* Planes: the vector area of the loops points along the outward normal. Loops made of one
  full circle are left out (in the files this repairs their direction is not reliable).
* Cylinders, cones, spheres and tori: along trusted edges (not full circles, not seams),
  ``normal x tangent`` points into the face when the surface normal is the outward one.
* Faces still open are settled from resolved neighbours across circular edges: a cylinder or
  cone that runs into the material of the plane it starts on is a hole, a coaxial cylinder or
  cone continuing it is hollow or solid alike, and a plane capping a boss faces away from it.

Faces on surfaces this module cannot evaluate (B-splines, offsets...) are left as they are.
"""

from __future__ import annotations

import math
import re
from dataclasses import dataclass
from pathlib import Path

Vec = tuple[float, float, float]

_ENTITY = re.compile(r"#(\d+)\s*=\s*([A-Z_0-9]+)\s*\((.*)\)\s*;\s*$", re.S)
_REVOLVED = ("CYLINDRICAL_SURFACE", "CONICAL_SURFACE")


# --------------------------------------------------------------------------- vectors

def _sub(a: Vec, b: Vec) -> Vec:
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def _add(a: Vec, b: Vec) -> Vec:
    return (a[0] + b[0], a[1] + b[1], a[2] + b[2])


def _mul(a: Vec, k: float) -> Vec:
    return (a[0] * k, a[1] * k, a[2] * k)


def _dot(a: Vec, b: Vec) -> float:
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def _cross(a: Vec, b: Vec) -> Vec:
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def _norm(a: Vec) -> float:
    return math.sqrt(_dot(a, a))


def _unit(a: Vec) -> Vec | None:
    n = _norm(a)
    return None if n < 1e-15 else (a[0] / n, a[1] / n, a[2] / n)


def _mean(pts: list[Vec]) -> Vec | None:
    if not pts:
        return None
    c = (0.0, 0.0, 0.0)
    for p in pts:
        c = _add(c, p)
    return _mul(c, 1.0 / len(pts))


# --------------------------------------------------------------------------- parsing

def _split_args(text: str) -> list:
    """Parse a STEP parameter list into nested Python lists of strings."""
    out: list = []
    stack = [out]
    tok = ""
    i = 0
    while i < len(text):
        c = text[i]
        if c == "'":
            j = i + 1
            while j < len(text):
                if text[j] == "'":
                    if j + 1 < len(text) and text[j + 1] == "'":
                        j += 2
                        continue
                    break
                j += 1
            tok += text[i : j + 1]
            i = j + 1
            continue
        if c == "(":
            new: list = []
            stack[-1].append(new)
            stack.append(new)
            tok = ""
        elif c == ")":
            if tok.strip():
                stack[-1].append(tok.strip())
            tok = ""
            stack.pop()
        elif c == ",":
            if tok.strip():
                stack[-1].append(tok.strip())
            tok = ""
        else:
            tok += c
        i += 1
    if tok.strip():
        stack[-1].append(tok.strip())
    return out


@dataclass
class Entity:
    type: str
    args: list
    line: int  # index in the statement list


def _ref(a: str) -> int:
    return int(a[1:])


class Step:
    def __init__(self, text: str):
        # Statements end with ';' (the strings in these files hold none).
        self.statements = re.split(r"(?<=;)", text)
        self.ent: dict[int, Entity] = {}
        for k, st in enumerate(self.statements):
            m = _ENTITY.search(st.strip())
            if m:
                try:
                    self.ent[int(m.group(1))] = Entity(m.group(2), _split_args(m.group(3)), k)
                except Exception:
                    continue

    def text(self) -> str:
        return "".join(self.statements)

    def point(self, i: int) -> Vec:
        c = self.ent[i].args[1]
        return (float(c[0]), float(c[1]), float(c[2]) if len(c) > 2 else 0.0)

    def direction(self, i: int) -> Vec:
        c = self.ent[i].args[1]
        v = (float(c[0]), float(c[1]), float(c[2]) if len(c) > 2 else 0.0)
        return _unit(v) or (0.0, 0.0, 1.0)

    def placement(self, i: int) -> tuple[Vec, Vec, Vec, Vec]:
        """Origin, x, y, z of an AXIS2_PLACEMENT_3D."""
        e = self.ent[i]
        o = self.point(_ref(e.args[1]))
        z = self.direction(_ref(e.args[2])) if e.args[2].startswith("#") else (0.0, 0.0, 1.0)
        if len(e.args) > 3 and e.args[3].startswith("#"):
            xr = self.direction(_ref(e.args[3]))
        else:
            xr = (1.0, 0.0, 0.0) if abs(z[0]) < 0.9 else (0.0, 1.0, 0.0)
        x = _unit(_sub(xr, _mul(z, _dot(xr, z)))) or (1.0, 0.0, 0.0)
        return o, x, _cross(z, x), z

    def vertex(self, i: int) -> Vec:
        return self.point(_ref(self.ent[i].args[1]))

    def surface(self, face: Entity) -> Entity | None:
        return self.ent.get(_ref(face.args[2]))

    def surface_normal(self, surf: Entity, p: Vec) -> Vec | None:
        """The surface's own normal at p (before the face's same_sense flag)."""
        t = surf.type
        if t == "PLANE":
            return self.placement(_ref(surf.args[1]))[3]
        if t in ("CYLINDRICAL_SURFACE", "CONICAL_SURFACE", "SPHERICAL_SURFACE", "TOROIDAL_SURFACE"):
            o, _x, _y, z = self.placement(_ref(surf.args[1]))
            d = _sub(p, o)
            r = _unit(_sub(d, _mul(z, _dot(d, z))))
            if t == "SPHERICAL_SURFACE":
                return _unit(d)
            if r is None:
                return None
            if t == "CYLINDRICAL_SURFACE":
                return r
            if t == "CONICAL_SURFACE":
                a = float(surf.args[3])
                if abs(a) > math.pi / 2:  # the file's angles are in degrees
                    a = math.radians(a)
                return _unit(_sub(_mul(r, math.cos(a)), _mul(z, math.sin(a))))
            return _unit(_sub(d, _mul(r, float(surf.args[2]))))  # torus
        return None

    def curve(self, cid: int) -> Entity | None:
        e = self.ent.get(cid)
        while e is not None and e.type in ("SURFACE_CURVE", "SEAM_CURVE"):
            e = self.ent.get(_ref(e.args[1]))
        return e

    def curve_samples(self, cid: int, a: Vec, b: Vec, closed: bool) -> list[Vec] | None:
        """Points along a curve from a to b in the curve's own direction."""
        e = self.curve(cid)
        if e is None:
            return None
        if e.type == "LINE":
            return [a, b]
        if e.type in ("CIRCLE", "ELLIPSE"):
            o, x, y, _z = self.placement(_ref(e.args[1]))

            def ang(p: Vec) -> float:
                d = _sub(p, o)
                return math.atan2(_dot(d, y), _dot(d, x))

            rx = float(e.args[2])
            ry = rx if e.type == "CIRCLE" else float(e.args[3])
            t0 = ang(a)
            span = (ang(b) - t0) % (2 * math.pi)
            if closed or span < 1e-9:
                span = 2 * math.pi
            n = 16
            return [
                _add(o, _add(_mul(x, rx * math.cos(t0 + span * k / n)), _mul(y, ry * math.sin(t0 + span * k / n))))
                for k in range(n + 1)
            ]
        if e.type.startswith("B_SPLINE_CURVE"):
            try:
                return [self.point(_ref(r)) for r in e.args[2]]
            except Exception:
                return None
        return None


# --------------------------------------------------------------------------- loops

@dataclass
class Segment:
    edge: int
    closed: bool  # a full circle: starts and ends on one vertex
    points: list[Vec]  # in travel order (the path along an arc is not trusted)
    line: bool = False

    @property
    def start(self) -> Vec:
        return self.points[0]


def _loop_segments(st: Step, loop_id: int, bound_sense: bool) -> list[Segment] | None:
    loop = st.ent.get(loop_id)
    if loop is None or loop.type != "EDGE_LOOP":
        return None
    segs: list[Segment] = []
    for oe_ref in loop.args[1]:
        oe = st.ent[_ref(oe_ref)]  # ORIENTED_EDGE('', *, *, edge, orientation)
        ec_id = _ref(oe.args[3])
        ec = st.ent[ec_id]  # EDGE_CURVE('', v1, v2, curve, same_sense)
        closed = ec.args[1] == ec.args[2]
        v1, v2 = st.vertex(_ref(ec.args[1])), st.vertex(_ref(ec.args[2]))
        forward = ec.args[4] == ".T."
        pts = st.curve_samples(_ref(ec.args[3]), v1 if forward else v2, v2 if forward else v1, closed)
        if pts is None:
            return None
        if not forward:
            pts = pts[::-1]  # now from v1 to v2
        if oe.args[4] != ".T.":
            pts = pts[::-1]
        c = st.curve(_ref(ec.args[3]))
        segs.append(Segment(ec_id, closed, pts, c is not None and c.type == "LINE"))
    if not bound_sense:
        segs = [Segment(s.edge, s.closed, s.points[::-1], s.line) for s in reversed(segs)]
    return segs


def _face_loops(st: Step, face: Entity) -> list[list[Segment]] | None:
    loops = []
    for b in face.args[1]:
        bound = st.ent[_ref(b)]
        segs = _loop_segments(st, _ref(bound.args[1]), bound.args[2] == ".T.")
        if segs is None:
            return None
        loops.append(segs)
    return loops


def _edges_of(st: Step, face: Entity) -> list[int]:
    out = []
    for b in face.args[1]:
        loop = st.ent[_ref(st.ent[_ref(b)].args[1])]
        for oe in loop.args[1]:
            out.append(_ref(st.ent[_ref(oe)].args[3]))
    return out


def _face_points(st: Step, face: Entity) -> list[Vec]:
    loops = _face_loops(st, face) or []
    return [s.start for segs in loops for s in segs]  # vertices: the arcs' paths are not trusted


def _sense_from_loops(st: Step, face: Entity) -> bool | None:
    surf = st.surface(face)
    loops = _face_loops(st, face)
    if surf is None or loops is None:
        return None
    if surf.type == "PLANE":
        # The polygon of the loop's vertices: their order is trusted, the arcs between them
        # are replaced by chords.
        area = (0.0, 0.0, 0.0)
        for segs in loops:
            if any(s.closed for s in segs):
                continue
            poly = [s.start for s in segs]
            for k in range(len(poly)):
                area = _add(area, _cross(poly[k], poly[(k + 1) % len(poly)]))
        n = st.surface_normal(surf, (0.0, 0.0, 0.0))
        if n is None or _norm(area) < 1e-9:
            return None
        return _dot(area, n) > 0
    uses: dict[int, int] = {}
    for segs in loops:
        for s in segs:
            uses[s.edge] = uses.get(s.edge, 0) + 1
    centre = _mean([s.start for segs in loops for s in segs])
    if centre is None:
        return None
    vote = 0.0
    for segs in loops:
        for s in segs:
            if not s.line or uses[s.edge] > 1:  # straight, non-seam edges only
                continue
            p, q = s.points[0], s.points[-1]
            t = _unit(_sub(q, p))
            n = st.surface_normal(surf, _mul(_add(p, q), 0.5))
            if t is None or n is None:
                continue
            vote += _dot(_cross(n, t), _sub(centre, p))
    if abs(vote) < 1e-9:
        return None
    return vote > 0


def _ring_centre(st: Step, ec_id: int) -> Vec | None:
    c = st.curve(_ref(st.ent[ec_id].args[3]))
    if c is None or c.type not in ("CIRCLE", "ELLIPSE"):
        return None
    return st.placement(_ref(c.args[1]))[0]


def _sense_from_neighbours(st: Step, fid: int, senses: dict[int, bool], faces_of_edge: dict[int, list[int]]) -> bool | None:
    """For cylinders and cones True is a boss (normal away from the axis), False a hole."""
    face = st.ent[fid]
    surf = st.surface(face)
    centre = _mean(_face_points(st, face))
    if surf is None or centre is None:
        return None
    vote = 0
    for ec_id in _edges_of(st, face):
        ring = _ring_centre(st, ec_id)
        if ring is None:
            continue
        for other in faces_of_edge.get(ec_id, []):
            if other == fid or other not in senses:
                continue
            osurf = st.surface(st.ent[other])
            if osurf is None:
                continue
            if surf.type in _REVOLVED and osurf.type == "PLANE":
                axis = st.placement(_ref(surf.args[1]))[3]
                n_plane = st.placement(_ref(osurf.args[1]))[3]
                outward = n_plane if senses[other] else _mul(n_plane, -1.0)
                along = _dot(_sub(centre, ring), axis) * _dot(axis, outward)
                if abs(along) > 1e-9:
                    vote += 1 if along > 0 else -1  # into the material: a hole
            elif surf.type in _REVOLVED and osurf.type in _REVOLVED:
                axis = st.placement(_ref(surf.args[1]))[3]
                o_axis = st.placement(_ref(osurf.args[1]))[3]
                if abs(abs(_dot(o_axis, axis)) - 1.0) < 1e-6:
                    vote += 1 if senses[other] else -1  # a step or chamfer of the same hole or boss
            elif surf.type == "PLANE" and osurf.type in _REVOLVED:
                n_plane = st.placement(_ref(surf.args[1]))[3]
                axis = st.placement(_ref(osurf.args[1]))[3]
                o_centre = _mean(_face_points(st, st.ent[other]))
                if o_centre is None or abs(abs(_dot(axis, n_plane)) - 1.0) > 1e-6:
                    continue
                away = _sub(ring, o_centre)  # the cap of a boss faces away from it
                outward = away if senses[other] else _mul(away, -1.0)  # a hole floor faces into it
                d = _dot(outward, n_plane)
                if abs(d) > 1e-9:
                    vote += 1 if d > 0 else -1
    if vote == 0:
        return None
    return vote > 0


# --------------------------------------------------------------------------- repair

@dataclass
class Fix:
    faces: int = 0
    flipped: int = 0
    unknown: int = 0
    circle_edges: int = 0  # circular edges whose direction was reversed


def _circle_edge_votes(st: Step, senses: dict[int, bool], faces: list[int]) -> dict[int, int]:
    """For circular edges on cylinders and cones: +1 keeps the edge's same_sense, -1 flips it.

    The face must lie on the left of the edge as travelled, seen from its outward normal: on
    a cylinder or cone that is along the axis, towards the rest of the face.
    """
    votes: dict[int, int] = {}
    for fid in faces:
        if fid not in senses:
            continue
        face = st.ent[fid]
        surf = st.surface(face)
        if surf is None or surf.type not in _REVOLVED:
            continue
        axis = st.placement(_ref(surf.args[1]))[3]
        centre = _mean(_face_points(st, face))
        if centre is None:
            continue
        for b in face.args[1]:
            bound = st.ent[_ref(b)]
            loop = st.ent[_ref(bound.args[1])]
            for oe_ref in loop.args[1]:
                oe = st.ent[_ref(oe_ref)]
                ec_id = _ref(oe.args[3])
                ec = st.ent[ec_id]
                c = st.curve(_ref(ec.args[3]))
                if c is None or c.type != "CIRCLE":
                    continue
                c_o, _x, _y, c_z = st.placement(_ref(c.args[1]))
                if abs(abs(_dot(c_z, axis)) - 1.0) > 1e-6:
                    continue  # not a ring around this face's axis
                p = st.vertex(_ref(ec.args[1]))
                inward = _dot(_sub(centre, p), axis)
                if abs(inward) < 1e-9:
                    continue
                sigma = (1 if oe.args[4] == ".T." else -1) * (1 if bound.args[2] == ".T." else -1)
                s_curve = 1 if ec.args[4] == ".T." else -1
                t = _mul(_cross(c_z, _sub(p, c_o)), sigma * s_curve)
                n = st.surface_normal(surf, p)
                if n is None:
                    continue
                if not senses[fid]:
                    n = _mul(n, -1.0)
                left = _dot(_cross(n, t), axis) * inward
                if abs(left) > 1e-12:
                    votes[ec_id] = votes.get(ec_id, 0) + (1 if left > 0 else -1)
    return votes


def repair_text(text: str) -> tuple[str, Fix]:
    st = Step(text)
    faces = [i for i, e in st.ent.items() if e.type == "ADVANCED_FACE"]
    faces_of_edge: dict[int, list[int]] = {}
    for i in faces:
        try:
            for ec in _edges_of(st, st.ent[i]):
                faces_of_edge.setdefault(ec, []).append(i)
        except Exception:
            continue
    senses: dict[int, bool] = {}
    pending: list[int] = []
    for i in faces:
        try:
            s = _sense_from_loops(st, st.ent[i])
        except Exception:
            s = None
        if s is None:
            pending.append(i)
        else:
            senses[i] = s
    changed = True
    while pending and changed:  # chains of faces settle one after another
        changed = False
        for i in list(pending):
            try:
                s = _sense_from_neighbours(st, i, senses, faces_of_edge)
            except Exception:
                s = None
            if s is not None:
                senses[i] = s
                pending.remove(i)
                changed = True
    fix = Fix(faces=len(faces), unknown=len(pending))
    for i, sense in senses.items():
        e = st.ent[i]
        flag = ".T." if sense else ".F."
        stmt = st.statements[e.line]
        body = stmt.rstrip()
        if body.endswith(f",{flag});"):
            continue
        new_body = re.sub(r",\s*\.[TF]\.\s*\)\s*;$", f",{flag});", body)
        if new_body != body:
            st.statements[e.line] = new_body + stmt[len(body):]
            fix.flipped += 1
    for ec_id, vote in _circle_edge_votes(st, senses, faces).items():
        if vote >= 0:
            continue
        e = st.ent[ec_id]
        stmt = st.statements[e.line]
        body = stmt.rstrip()
        flag = ".F." if e.args[4] == ".T." else ".T."
        new_body = re.sub(r",\s*\.[TF]\.\s*\)\s*;$", f",{flag});", body)
        if new_body != body:
            st.statements[e.line] = new_body + stmt[len(body):]
            fix.circle_edges += 1
    return st.text(), fix


_POINT3 = re.compile(r"(CARTESIAN_POINT|DIRECTION)\s*\(\s*('(?:[^']|'')*')\s*,\s*\(\s*([^,()]+)\s*,\s*([^,()]+)\s*,\s*([^,()]+)\s*\)\s*\)")


def rotate_text(text: str, m: tuple[float, ...]) -> str:
    """Turn every 3D point and direction of a STEP file by the rotation m (row-major).

    Two-dimensional points (curves in a surface's parameter space) are left alone. With a
    proper rotation the orientation of every face and edge is unchanged.
    """
    def turn(match: re.Match) -> str:
        x, y, z = (float(match.group(k)) for k in (3, 4, 5))
        p = (m[0] * x + m[1] * y + m[2] * z, m[3] * x + m[4] * y + m[5] * z, m[6] * x + m[7] * y + m[8] * z)
        return f"{match.group(1)}({match.group(2)},({p[0]!r},{p[1]!r},{p[2]!r}))"

    return _POINT3.sub(turn, text)


def repair_file(path: Path) -> Fix:
    out, fix = repair_text(path.read_text(encoding="latin-1"))
    path.write_text(out, encoding="latin-1")
    return fix

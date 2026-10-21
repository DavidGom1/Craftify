"""Repair a STEP file's orientation flags using the source CAD's own display mesh.

cadmpeg 0.6.2 writes SolidWorks parts with wrong orientation flags: every face says it is
reversed, most edges run against their curves, and arcs can name the wrong side of their
circle. SolidCraft then fills holes with discs, leaves loose cylinders or draws faces that
look see-through. The display mesh SolidWorks saved in the part has outward normals and lies
on the true surfaces, so it settles each flag directly:

* face same_sense: the mesh's outward normals over the face against the surface's normal;
* line and B-spline edges: the curve's own direction against the edge's vertex order (exact);
* arcs: of the two arcs between an edge's vertices, the one whose middle lies on the mesh;
* full circles: travelled so the face is on their left, outer loops counter-clockwise.

Faces the mesh cannot settle (no triangles found on them) keep the rule-based answer from
``step_fix``.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path

from . import step_fix as sf
from .mesh import Mesh

Vec = tuple[float, float, float]


@dataclass
class MeshFix:
    faces: int = 0
    faces_from_mesh: int = 0
    faces_from_rules: int = 0
    faces_unknown: int = 0
    faces_flipped: int = 0
    curve_edges: int = 0  # lines and B-splines re-directed
    arcs: int = 0  # arcs moved to the other side of their circle
    circles: int = 0  # full circles re-directed


def _tri_normal(a: Vec, b: Vec, c: Vec) -> Vec | None:
    return sf._unit(sf._cross(sf._sub(b, a), sf._sub(c, a)))


def _surface_distance(st: sf.Step, surf: sf.Entity, p: Vec) -> float | None:
    t = surf.type
    if t == "PLANE":
        o, _x, _y, z = st.placement(sf._ref(surf.args[1]))
        return abs(sf._dot(sf._sub(p, o), z))
    if t in ("CYLINDRICAL_SURFACE", "CONICAL_SURFACE", "SPHERICAL_SURFACE", "TOROIDAL_SURFACE"):
        o, _x, _y, z = st.placement(sf._ref(surf.args[1]))
        d = sf._sub(p, o)
        axial = sf._dot(d, z)
        radial = sf._norm(sf._sub(d, sf._mul(z, axial)))
        if t == "CYLINDRICAL_SURFACE":
            return abs(radial - float(surf.args[2]))
        if t == "SPHERICAL_SURFACE":
            return abs(sf._norm(d) - float(surf.args[2]))
        if t == "CONICAL_SURFACE":
            a = float(surf.args[3])
            if abs(a) > math.pi / 2:
                a = math.radians(a)
            r_at = float(surf.args[2]) + axial * math.tan(a)
            return abs(radial - r_at) * math.cos(a)
        major, minor = float(surf.args[2]), float(surf.args[3])
        return abs(math.hypot(radial - major, axial) - minor)
    return None


def _bspline_normal(st: sf.Step, surf: sf.Entity, p: Vec) -> tuple[Vec | None, float]:
    """Normal of a B-spline surface near p from its control net, and p's distance to the net."""
    try:
        grid = [[st.point(sf._ref(r)) for r in row] for row in surf.args[3]]
    except Exception:
        return None, float("inf")
    best, bi, bj = float("inf"), 0, 0
    for i, row in enumerate(grid):
        for j, q in enumerate(row):
            d = sf._norm(sf._sub(q, p))
            if d < best:
                best, bi, bj = d, i, j
    n_u, n_v = len(grid), len(grid[0])
    i0, i1 = max(0, bi - 1), min(n_u - 1, bi + 1)
    j0, j1 = max(0, bj - 1), min(n_v - 1, bj + 1)
    du = sf._sub(grid[i1][bj], grid[i0][bj])
    dv = sf._sub(grid[bi][j1], grid[bi][j0])
    return sf._unit(sf._cross(du, dv)), best


def _face_box(st: sf.Step, face: sf.Entity, pad: float) -> tuple[Vec, Vec] | None:
    pts = []
    for b in face.args[1]:
        loop = st.ent[sf._ref(st.ent[sf._ref(b)].args[1])]
        for oe in loop.args[1]:
            ec = st.ent[sf._ref(st.ent[sf._ref(oe)].args[3])]
            v1, v2 = st.vertex(sf._ref(ec.args[1])), st.vertex(sf._ref(ec.args[2]))
            pts += [v1, v2]
            c = st.curve(sf._ref(ec.args[3]))
            if c is not None and c.type in ("CIRCLE", "ELLIPSE"):
                o, x, y, _z = st.placement(sf._ref(c.args[1]))
                r = float(c.args[2])
                pts += [sf._add(o, sf._add(sf._mul(x, r * math.cos(t)), sf._mul(y, r * math.sin(t))))
                        for t in (k * math.pi / 4 for k in range(8))]
            elif c is not None and c.type.startswith("B_SPLINE_CURVE"):
                try:
                    pts += [st.point(sf._ref(r)) for r in c.args[2]]
                except Exception:
                    pass
    if not pts:
        return None
    lo = tuple(min(p[k] for p in pts) - pad for k in range(3))
    hi = tuple(max(p[k] for p in pts) + pad for k in range(3))
    return lo, hi  # type: ignore[return-value]


def _in_box(p: Vec, box: tuple[Vec, Vec]) -> bool:
    lo, hi = box
    return all(lo[k] <= p[k] <= hi[k] for k in range(3))


def _face_sense_from_mesh(st: sf.Step, face: sf.Entity, mesh: Mesh, centroids: list[tuple[Vec, Vec]]) -> bool | None:
    surf = st.surface(face)
    if surf is None:
        return None
    box = _face_box(st, face, pad=0.5)
    if box is None:
        return None
    vote = 0.0
    for c, n in centroids:
        if not _in_box(c, box):
            continue
        if surf.type.startswith("B_SPLINE_SURFACE"):
            sn, d = _bspline_normal(st, surf, c)
            if sn is None or d > 2.0:
                continue
        else:
            d = _surface_distance(st, surf, c)
            if d is None:
                return None
            r = float(surf.args[2]) if surf.type != "PLANE" else 0.0
            if d > max(0.02, 0.03 * r):
                continue
            sn = st.surface_normal(surf, c)
            if sn is None:
                continue
        dot = sf._dot(sn, n)
        if abs(dot) < 0.5:
            continue  # the triangle is not along this surface there
        vote += 1 if dot > 0 else -1
    if vote == 0:
        return None
    return vote > 0


def _arc_mid(st: sf.Step, circle: sf.Entity, a: Vec, b: Vec) -> Vec:
    """Middle of the counter-clockwise arc (about the circle's axis) from a to b."""
    o, x, y, _z = st.placement(sf._ref(circle.args[1]))
    rx = float(circle.args[2])
    ry = rx if circle.type == "CIRCLE" else float(circle.args[3])

    def ang(p: Vec) -> float:
        d = sf._sub(p, o)
        return math.atan2(sf._dot(d, y), sf._dot(d, x))

    t0 = ang(a)
    span = (ang(b) - t0) % (2 * math.pi)
    t = t0 + span / 2
    return sf._add(o, sf._add(sf._mul(x, rx * math.cos(t)), sf._mul(y, ry * math.sin(t))))


def _loop_extent(st: sf.Step, bound: sf.Entity) -> float:
    pts = []
    loop = st.ent[sf._ref(bound.args[1])]
    for oe in loop.args[1]:
        ec = st.ent[sf._ref(st.ent[sf._ref(oe)].args[3])]
        pts.append(st.vertex(sf._ref(ec.args[1])))
        c = st.curve(sf._ref(ec.args[3]))
        if c is not None and c.type in ("CIRCLE", "ELLIPSE"):
            return 2 * float(c.args[2]) if len(loop.args[1]) == 1 else max(2 * float(c.args[2]), 0.0)
    if len(pts) < 2:
        return 0.0
    return max(sf._norm(sf._sub(p, q)) for p in pts for q in pts)


def repair_text(text: str, mesh: Mesh, faces_mode: str = "rules", lines: bool = True,
                arcs: bool = True, circles: bool = True) -> tuple[str, MeshFix]:
    """faces_mode: "rules" (loop-consistent, from step_fix), "mesh" (the mesh's normals) or
    "mesh_loops" (the mesh's answer, turned to agree with the loops when they are all reversed)."""
    fix = MeshFix()
    ruled_text, _ = sf.repair_text(text)  # rule-based face flags and circle votes
    st = sf.Step(ruled_text if faces_mode == "rules" else text)

    # 1. lines and B-splines: exact
    if lines:
        for e in list(st.ent.values()):
            if e.type == "EDGE_CURVE":
                g = _geometric_edge_sense(st, e)
                if g is not None and _set_last_flag(st, e, g):
                    fix.curve_edges += 1

    # 2. faces
    faces = [i for i, e in st.ent.items() if e.type == "ADVANCED_FACE"]
    fix.faces = len(faces)
    senses: dict[int, bool] = {i: st.ent[i].args[3] == ".T." for i in faces}
    if faces_mode != "rules":
        rst = sf.Step(ruled_text)
        centroids = []
        for a, b, c in mesh.triangles:
            n = _tri_normal(a, b, c)
            if n is not None:
                centroids.append(((sum(p[0] for p in (a, b, c)) / 3, sum(p[1] for p in (a, b, c)) / 3, sum(p[2] for p in (a, b, c)) / 3), n))
        from_mesh: dict[int, bool] = {}
        for i in faces:
            try:
                s = _face_sense_from_mesh(st, st.ent[i], mesh, centroids)
            except Exception:
                s = None
            if s is not None:
                from_mesh[i] = s
        flip_all = False
        if faces_mode == "mesh_loops" and from_mesh:
            agree = sum(1 for i, s in from_mesh.items() if s == (rst.ent[i].args[3] == ".T."))
            flip_all = agree < len(from_mesh) / 2  # the loops run reversed throughout
        for i in faces:
            if i in from_mesh:
                senses[i] = (not from_mesh[i]) if flip_all else from_mesh[i]
                fix.faces_from_mesh += 1
            else:
                senses[i] = rst.ent[i].args[3] == ".T."
                fix.faces_from_rules += 1
        for i, s in senses.items():
            if _set_last_flag(st, st.ent[i], s):
                fix.faces_flipped += 1
    if not arcs and not circles:
        return st.text(), fix

    # 3. arcs: the side whose middle lies on the mesh
    for e in list(st.ent.values()):
        if not arcs:
            break
        if e.type != "EDGE_CURVE" or e.args[1] == e.args[2]:
            continue
        c = st.curve(sf._ref(e.args[3]))
        if c is None or c.type not in ("CIRCLE", "ELLIPSE"):
            continue
        v1, v2 = st.vertex(sf._ref(e.args[1])), st.vertex(sf._ref(e.args[2]))
        r = float(c.args[2])
        mid_t = _arc_mid(st, c, v1, v2)  # same_sense .T.: counter-clockwise from v1 to v2
        mid_f = _arc_mid(st, c, v2, v1)  # .F.: the other side
        dt = mesh.signed_distance(mid_t, reach=r + 1.0)
        df = mesh.signed_distance(mid_f, reach=r + 1.0)
        dt = abs(dt) if dt is not None else float("inf")
        df = abs(df) if df is not None else float("inf")
        if min(dt, df) == float("inf") or abs(dt - df) < 1e-6:
            continue
        if _set_last_flag(st, e, dt < df):
            fix.arcs += 1

    # 4. full circles: the face on their left
    if not circles:
        return st.text(), fix
    faces_of_edge: dict[int, list[int]] = {}
    for i in faces:
        for ec in sf._edges_of(st, st.ent[i]):
            faces_of_edge.setdefault(ec, []).append(i)
    votes = sf._circle_edge_votes(st, senses, faces)  # cylinders and cones
    for i in faces:
        face = st.ent[i]
        surf = st.surface(face)
        if surf is None or surf.type != "PLANE" or i not in senses:
            continue
        n_out = st.placement(sf._ref(surf.args[1]))[3]
        if not senses[i]:
            n_out = sf._mul(n_out, -1.0)
        bounds = [st.ent[sf._ref(b)] for b in face.args[1]]
        extents = [_loop_extent(st, b) for b in bounds]
        biggest = max(range(len(bounds)), key=lambda k: extents[k]) if bounds else -1
        for k, bound in enumerate(bounds):
            loop = st.ent[sf._ref(bound.args[1])]
            if len(loop.args[1]) != 1:
                continue
            oe = st.ent[sf._ref(loop.args[1][0])]
            ec_id = sf._ref(oe.args[3])
            ec = st.ent[ec_id]
            c = st.curve(sf._ref(ec.args[3]))
            if c is None or c.type != "CIRCLE" or ec.args[1] != ec.args[2]:
                continue
            _o, _x, _y, cz = st.placement(sf._ref(c.args[1]))
            sigma = (1 if oe.args[4] == ".T." else -1) * (1 if bound.args[2] == ".T." else -1)
            s_curve = 1 if ec.args[4] == ".T." else -1
            ccw = sf._dot(cz, n_out) * sigma * s_curve > 0
            want_ccw = k == biggest and len(bounds) == 1 or (k == biggest and len(bounds) > 1)
            votes[ec_id] = votes.get(ec_id, 0) + (1 if ccw == want_ccw else -1)
    for ec_id, vote in votes.items():
        e = st.ent[ec_id]
        if vote < 0 and e.args[1] == e.args[2]:  # arcs were settled from the mesh already
            if _set_last_flag(st, e, e.args[4] != ".T."):
                fix.circles += 1
    return st.text(), fix


def repair_file(path: Path, mesh: Mesh) -> MeshFix:
    out, fix = repair_text(path.read_text(encoding="latin-1"), mesh)
    path.write_text(out, encoding="latin-1")
    return fix


# --------------------------------------------------------------------------- flags

def _geometric_edge_sense(st: sf.Step, ec: sf.Entity) -> bool | None:
    """Does the edge run along its curve's own direction? Exact for lines and B-splines."""
    c = st.curve(sf._ref(ec.args[3]))
    if c is None:
        return None
    v1, v2 = st.vertex(sf._ref(ec.args[1])), st.vertex(sf._ref(ec.args[2]))
    if c.type == "LINE":
        vec = st.ent.get(sf._ref(c.args[2]))
        if vec is None or vec.type != "VECTOR":
            return None
        d = sf._dot(st.direction(sf._ref(vec.args[1])), sf._sub(v2, v1))
        return None if abs(d) < 1e-12 else d > 0
    if c.type.startswith("B_SPLINE_CURVE"):
        if ec.args[1] == ec.args[2]:
            return None
        try:
            first, last = st.point(sf._ref(c.args[2][0])), st.point(sf._ref(c.args[2][-1]))
        except Exception:
            return None
        same = sf._norm(sf._sub(first, v1)) + sf._norm(sf._sub(last, v2))
        rev = sf._norm(sf._sub(first, v2)) + sf._norm(sf._sub(last, v1))
        if abs(same - rev) < 1e-9:
            return None
        return same < rev
    return None


def _set_last_flag(st: sf.Step, e: sf.Entity, value: bool) -> bool:
    """Rewrite an entity's last argument (.T./.F.); True when it changed."""
    import re

    flag = ".T." if value else ".F."
    stmt = st.statements[e.line]
    body = stmt.rstrip()
    if body.endswith(f",{flag});"):
        return False
    new_body = re.sub(r",\s*\.[TF]\.\s*\)\s*;$", f",{flag});", body)
    if new_body == body:
        return False
    st.statements[e.line] = new_body + stmt[len(body):]
    e.args[-1] = flag
    return True

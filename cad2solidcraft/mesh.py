"""Triangle meshes for checking a rebuilt part against the source CAD's own display mesh.

The source mesh is the ground truth for a conversion: its volume and box say whether a
rebuilt timeline matches, and the signed distance from a point to it says whether a point
of the rebuilt model was cut away in the source (how edges of chamfers and fillets are found).
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field

Vec = tuple[float, float, float]


def _sub(a: Vec, b: Vec) -> Vec:
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def _dot(a: Vec, b: Vec) -> float:
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def _cross(a: Vec, b: Vec) -> Vec:
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def _add(a: Vec, b: Vec) -> Vec:
    return (a[0] + b[0], a[1] + b[1], a[2] + b[2])


def _mul(a: Vec, k: float) -> Vec:
    return (a[0] * k, a[1] * k, a[2] * k)


def _closest_on_triangle(p: Vec, a: Vec, b: Vec, c: Vec) -> Vec:
    """Closest point to p on triangle abc (Ericson, Real-Time Collision Detection 5.1.5)."""
    ab, ac, ap = _sub(b, a), _sub(c, a), _sub(p, a)
    d1, d2 = _dot(ab, ap), _dot(ac, ap)
    if d1 <= 0 and d2 <= 0:
        return a
    bp = _sub(p, b)
    d3, d4 = _dot(ab, bp), _dot(ac, bp)
    if d3 >= 0 and d4 <= d3:
        return b
    vc = d1 * d4 - d3 * d2
    if vc <= 0 and d1 >= 0 and d3 <= 0:
        return _add(a, _mul(ab, d1 / (d1 - d3)))
    cp = _sub(p, c)
    d5, d6 = _dot(ab, cp), _dot(ac, cp)
    if d6 >= 0 and d5 <= d6:
        return c
    vb = d5 * d2 - d1 * d6
    if vb <= 0 and d2 >= 0 and d6 <= 0:
        return _add(a, _mul(ac, d2 / (d2 - d6)))
    va = d3 * d6 - d5 * d4
    if va <= 0 and (d4 - d3) >= 0 and (d5 - d6) >= 0:
        return _add(b, _mul(_sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))))
    denom = 1.0 / (va + vb + vc)
    return _add(a, _add(_mul(ab, vb * denom), _mul(ac, vc * denom)))


def _ray_hits(o: Vec, d: Vec, a: Vec, b: Vec, c: Vec) -> bool:
    """Does the ray o + t·d (t > 0) cross triangle abc? (Moller-Trumbore)"""
    e1, e2 = _sub(b, a), _sub(c, a)
    h = _cross(d, e2)
    det = _dot(e1, h)
    if abs(det) < 1e-14:
        return False
    inv = 1.0 / det
    s = _sub(o, a)
    u = _dot(s, h) * inv
    if u < 0.0 or u > 1.0:
        return False
    q = _cross(s, e1)
    v = _dot(d, q) * inv
    if v < 0.0 or u + v > 1.0:
        return False
    return _dot(e2, q) * inv > 1e-9


@dataclass
class Mesh:
    """Triangles wound counter-clockwise seen from outside (outward normals)."""

    triangles: list[tuple[Vec, Vec, Vec]] = field(default_factory=list)
    _grid: dict | None = None
    _cell: float = 5.0

    def add(self, a: Vec, b: Vec, c: Vec, outward: Vec | None = None) -> None:
        g = _cross(_sub(b, a), _sub(c, a))
        if _dot(g, g) < 1e-24:
            return  # degenerate
        if outward is not None and _dot(g, outward) < 0:
            b, c = c, b
        self.triangles.append((a, b, c))
        self._grid = None

    def transformed(self, m: tuple[float, ...]) -> "Mesh":
        """The mesh with every point multiplied by the 3x3 matrix m (row-major, column vectors)."""
        def t(p: Vec) -> Vec:
            return (m[0] * p[0] + m[1] * p[1] + m[2] * p[2], m[3] * p[0] + m[4] * p[1] + m[5] * p[2],
                    m[6] * p[0] + m[7] * p[1] + m[8] * p[2])
        out = Mesh()
        out.triangles = [(t(a), t(b), t(c)) for a, b, c in self.triangles]
        return out

    def volume(self) -> float:
        return sum(_dot(a, _cross(b, c)) for a, b, c in self.triangles) / 6.0

    def area(self) -> float:
        return sum(math.sqrt(_dot(g, g)) for g in (_cross(_sub(b, a), _sub(c, a)) for a, b, c in self.triangles)) / 2.0

    def bbox(self) -> tuple[Vec, Vec]:
        xs = [p for tri in self.triangles for p in tri]
        lo = (min(p[0] for p in xs), min(p[1] for p in xs), min(p[2] for p in xs))
        hi = (max(p[0] for p in xs), max(p[1] for p in xs), max(p[2] for p in xs))
        return lo, hi

    # ---------------------------------------------------------------- inside test
    _axis_grids: dict | None = None

    def _build_axis_grids(self) -> None:
        """For rays along each axis: triangles bucketed by their shadow on the other two."""
        grids = {}
        s = self._cell
        for a in range(3):
            b, c = (a + 1) % 3, (a + 2) % 3
            g: dict[tuple[int, int], list[int]] = {}
            for i, tri in enumerate(self.triangles):
                lo_b = math.floor(min(p[b] for p in tri) / s)
                hi_b = math.floor(max(p[b] for p in tri) / s)
                lo_c = math.floor(min(p[c] for p in tri) / s)
                hi_c = math.floor(max(p[c] for p in tri) / s)
                for x in range(lo_b, hi_b + 1):
                    for y in range(lo_c, hi_c + 1):
                        g.setdefault((x, y), []).append(i)
            grids[a] = g
        self._axis_grids = grids

    def inside(self, p: Vec) -> bool:
        """Is p inside the closed mesh? Ray parity along the three axes, majority vote."""
        if self._axis_grids is None:
            self._build_axis_grids()
        assert self._axis_grids is not None
        s = self._cell
        votes = 0
        q = (p[0] + 1.37e-7, p[1] + 2.91e-7, p[2] + 0.53e-7)  # off exact edges and vertices
        for a in range(3):
            b, c = (a + 1) % 3, (a + 2) % 3
            cell = (math.floor(q[b] / s), math.floor(q[c] / s))
            hits = 0
            for i in self._axis_grids[a].get(cell, ()):
                t0, t1, t2 = self.triangles[i]
                # 2D point-in-triangle on the (b, c) shadow, then the crossing must lie ahead on a
                d = (t1[b] - t0[b]) * (t2[c] - t0[c]) - (t2[b] - t0[b]) * (t1[c] - t0[c])
                if abs(d) < 1e-15:
                    continue
                w1 = ((q[b] - t0[b]) * (t2[c] - t0[c]) - (t2[b] - t0[b]) * (q[c] - t0[c])) / d
                w2 = ((t1[b] - t0[b]) * (q[c] - t0[c]) - (q[b] - t0[b]) * (t1[c] - t0[c])) / d
                if w1 < 0 or w2 < 0 or w1 + w2 > 1:
                    continue
                x = t0[a] + w1 * (t1[a] - t0[a]) + w2 * (t2[a] - t0[a])
                if x > q[a]:
                    hits += 1
            votes += hits % 2
        return votes >= 2

    # ---------------------------------------------------------------- distance queries
    def _build_grid(self) -> None:
        grid: dict[tuple[int, int, int], list[int]] = {}
        s = self._cell
        for i, (a, b, c) in enumerate(self.triangles):
            lo = [math.floor(min(a[k], b[k], c[k]) / s) for k in range(3)]
            hi = [math.floor(max(a[k], b[k], c[k]) / s) for k in range(3)]
            for x in range(lo[0], hi[0] + 1):
                for y in range(lo[1], hi[1] + 1):
                    for z in range(lo[2], hi[2] + 1):
                        grid.setdefault((x, y, z), []).append(i)
        self._grid = grid

    def signed_distance(self, p: Vec, reach: float) -> float | None:
        """Distance from p to the surface, positive outside, negative inside.

        Only triangles within ``reach`` are looked at; None when there is none.
        """
        if self._grid is None:
            self._build_grid()
        assert self._grid is not None
        s = self._cell
        lo = [math.floor((p[k] - reach) / s) for k in range(3)]
        hi = [math.floor((p[k] + reach) / s) for k in range(3)]
        seen: set[int] = set()
        best = None
        best_d2 = reach * reach
        for x in range(lo[0], hi[0] + 1):
            for y in range(lo[1], hi[1] + 1):
                for z in range(lo[2], hi[2] + 1):
                    for i in self._grid.get((x, y, z), ()):
                        if i in seen:
                            continue
                        seen.add(i)
                        a, b, c = self.triangles[i]
                        q = _closest_on_triangle(p, a, b, c)
                        d = _sub(p, q)
                        d2 = _dot(d, d)
                        if d2 <= best_d2:
                            best_d2, best = d2, (i, q)
        if best is None:
            return None
        i, q = best
        dist = math.sqrt(best_d2)
        if dist < 1e-9:
            return 0.0
        # Sign: the side of the nearest triangles. Near an edge or corner several triangles are
        # equally near; their summed normals give the correct side for a closed mesh.
        normal = (0.0, 0.0, 0.0)
        for j in seen:
            a, b, c = self.triangles[j]
            qq = _closest_on_triangle(p, a, b, c)
            dd = _sub(p, qq)
            if abs(math.sqrt(_dot(dd, dd)) - dist) < 1e-6 * max(1.0, dist):
                g = _cross(_sub(b, a), _sub(c, a))
                n = math.sqrt(_dot(g, g))
                if n > 0:
                    normal = _add(normal, _mul(g, 1.0 / n))
        side = _dot(_sub(p, q), normal)
        return dist if side >= 0 else -dist

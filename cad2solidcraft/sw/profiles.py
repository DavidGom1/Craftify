"""Sketch regions, and choosing an extrusion's plane, side and regions from the source mesh.

cadmpeg gives each sketch's curves but not which closed regions the extrusion used, and some
sketch planes come with a wrong orientation. The source part's own display mesh decides:
an added region's material is there in the finished part, and a cut region is empty while
the material around it is there.
"""

from __future__ import annotations

import itertools
import math
from dataclasses import dataclass, field

from ..mesh import Mesh

UV = tuple[float, float]
Vec = tuple[float, float, float]


@dataclass
class Loop:
    points: list[UV]  # closed polygon (arcs sampled), last point not repeated
    circle: tuple[UV, float] | None = None  # (centre, radius) when the loop is one circle

    def area(self) -> float:
        a = 0.0
        p = self.points
        for k in range(len(p)):
            a += p[k][0] * p[(k + 1) % len(p)][1] - p[(k + 1) % len(p)][0] * p[k][1]
        return a / 2.0

    def contains(self, q: UV) -> bool:
        if self.circle is not None:
            (cu, cv), r = self.circle
            return math.hypot(q[0] - cu, q[1] - cv) < r
        inside = False
        p = self.points
        j = len(p) - 1
        for i in range(len(p)):
            (xi, yi), (xj, yj) = p[i], p[j]
            if (yi > q[1]) != (yj > q[1]) and q[0] < (xj - xi) * (q[1] - yi) / (yj - yi) + xi:
                inside = not inside
            j = i
        return inside

    def bbox(self) -> tuple[UV, UV]:
        xs = [p[0] for p in self.points]
        ys = [p[1] for p in self.points]
        return (min(xs), min(ys)), (max(xs), max(ys))

    def outside_ring(self, delta: float) -> list[UV]:
        """Points just outside the loop, around its whole length."""
        if self.circle is not None:
            (cu, cv), r = self.circle
            return [(cu + (r + delta) * math.cos(a), cv + (r + delta) * math.sin(a)) for a in (i * math.pi / 4 for i in range(8))]
        out = []
        p = self.points
        ccw = self.area() > 0
        for k in range(len(p)):
            a, b = p[k], p[(k + 1) % len(p)]
            dx, dy = b[0] - a[0], b[1] - a[1]
            n = math.hypot(dx, dy)
            if n < 1e-9:
                continue
            nx, ny = (dy / n, -dx / n) if ccw else (-dy / n, dx / n)  # right of travel on a ccw loop: outside
            out.append(((a[0] + b[0]) / 2 + nx * delta, (a[1] + b[1]) / 2 + ny * delta))
        return out


@dataclass
class Region:
    outer: Loop
    holes: list[Loop] = field(default_factory=list)
    samples: list[UV] = field(default_factory=list)

    def contains(self, q: UV) -> bool:
        return self.outer.contains(q) and not any(h.contains(q) for h in self.holes)


def loops_from_curves(curves: list[dict], arc_mid) -> list[Loop]:
    """Closed loops from sketch curves: circles, and chains of lines and arcs end to end."""
    loops: list[Loop] = []
    segs = []  # (start, end, interior points)
    for e in curves:
        g = e["geometry"]
        if e.get("construction"):
            continue
        if g["kind"] == "circle":
            c = (float(g["center"]["u"]), float(g["center"]["v"]))
            r = float(g["radius"])
            pts = [(c[0] + r * math.cos(2 * math.pi * k / 48), c[1] + r * math.sin(2 * math.pi * k / 48)) for k in range(48)]
            loops.append(Loop(pts, (c, r)))
        elif g["kind"] == "line":
            a = (float(g["start"]["u"]), float(g["start"]["v"]))
            b = (float(g["end"]["u"]), float(g["end"]["v"]))
            segs.append((a, b, []))
        elif g["kind"] == "arc":
            p0, mid, p1 = arc_mid(e)
            c = (float(g["center"]["u"]), float(g["center"]["v"]))
            r = float(g["radius"])
            a0 = math.atan2(p0[1] - c[1], p0[0] - c[0])
            am = math.atan2(mid[1] - c[1], mid[0] - c[0])
            a1 = math.atan2(p1[1] - c[1], p1[0] - c[0])
            ccw = (am - a0) % (2 * math.pi) < (a1 - a0) % (2 * math.pi)
            span = (a1 - a0) % (2 * math.pi) if ccw else -((a0 - a1) % (2 * math.pi))
            inner = [(c[0] + r * math.cos(a0 + span * k / 12), c[1] + r * math.sin(a0 + span * k / 12)) for k in range(1, 12)]
            segs.append((tuple(p0), tuple(p1), inner))

    def key(p: UV) -> tuple[int, int]:
        return (round(p[0] * 1e4), round(p[1] * 1e4))

    unused = list(range(len(segs)))
    while unused:
        first = unused.pop(0)
        a, b, inner = segs[first]
        chain = [a] + inner
        cur = b
        start = key(a)
        closed = False
        while True:
            if key(cur) == start:
                closed = True
                break
            nxt = None
            for i in unused:
                s0, s1, sin = segs[i]
                if key(s0) == key(cur):
                    nxt = (i, s0, s1, sin)
                    break
                if key(s1) == key(cur):
                    nxt = (i, s1, s0, list(reversed(sin)))
                    break
            if nxt is None:
                break
            i, s0, s1, sin = nxt
            unused.remove(i)
            chain += [s0] + sin
            cur = s1
        if closed and len(chain) >= 3:
            loops.append(Loop(chain))
    return loops


def regions_from_loops(loops: list[Loop]) -> list[Region]:
    """Each loop's inside minus the loops directly inside it."""
    def inside_of(a: Loop, b: Loop) -> bool:  # is a inside b?
        return a is not b and all(b.contains(p) for p in a.points[:: max(1, len(a.points) // 8)])

    regions = []
    for lp in loops:
        children = [c for c in loops if inside_of(c, lp)]
        direct = [c for c in children if not any(inside_of(c, o) for o in children if o is not c)]
        reg = Region(lp, direct)
        reg.samples = _region_samples(reg)
        if reg.samples:
            regions.append(reg)
    return regions


def _region_samples(reg: Region, wanted: int = 5) -> list[UV]:
    if reg.outer.circle is not None and not reg.holes:
        (cu, cv), r = reg.outer.circle
        return [(cu, cv)] + [(cu + 0.5 * r * math.cos(a), cv + 0.5 * r * math.sin(a)) for a in (0, math.pi / 2, math.pi, 3 * math.pi / 2)]
    (x0, y0), (x1, y1) = reg.outer.bbox()
    best: list[tuple[float, UV]] = []
    n = 24
    edges = [reg.outer] + reg.holes
    for i, j in itertools.product(range(1, n), range(1, n)):
        q = (x0 + (x1 - x0) * i / n, y0 + (y1 - y0) * j / n)
        if not reg.contains(q):
            continue
        clearance = min(_dist_to_loop(q, lp) for lp in edges)
        best.append((clearance, q))
    best.sort(key=lambda t: -t[0])
    picked: list[UV] = []
    for _c, q in best:
        if all(math.hypot(q[0] - p[0], q[1] - p[1]) > 0.15 * max(x1 - x0, y1 - y0) for p in picked):
            picked.append(q)
        if len(picked) >= wanted:
            break
    return picked


def _dist_to_loop(q: UV, lp: Loop) -> float:
    if lp.circle is not None:
        (cu, cv), r = lp.circle
        return abs(math.hypot(q[0] - cu, q[1] - cv) - r)
    best = float("inf")
    p = lp.points
    for k in range(len(p)):
        a, b = p[k], p[(k + 1) % len(p)]
        dx, dy = b[0] - a[0], b[1] - a[1]
        L = dx * dx + dy * dy
        t = 0.0 if L == 0 else max(0.0, min(1.0, ((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / L))
        best = min(best, math.hypot(q[0] - a[0] - t * dx, q[1] - a[1] - t * dy))
    return best


# --------------------------------------------------------------------------- frames

@dataclass
class Frame:
    origin: Vec
    x: Vec
    y: Vec

    @property
    def normal(self) -> Vec:
        x, y = self.x, self.y
        return (x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0])

    def point(self, q: UV, depth: float = 0.0) -> Vec:
        n = self.normal
        return tuple(self.origin[k] + q[0] * self.x[k] + q[1] * self.y[k] + depth * n[k] for k in range(3))  # type: ignore[return-value]


def candidate_frames(given: Frame) -> list[Frame]:
    """The given frame first, then every axis-aligned frame with the same origin."""
    axes = [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)]
    out = [given]
    for i, j in itertools.permutations(range(3), 2):
        for si, sj in itertools.product((1.0, -1.0), repeat=2):
            x = tuple(si * c for c in axes[i])
            y = tuple(sj * c for c in axes[j])
            out.append(Frame(given.origin, x, y))  # type: ignore[arg-type]
    return out


@dataclass
class Choice:
    frame: Frame
    direction: int  # +1 along the frame's normal, -1 against it
    regions: list[Region]
    score: float
    perfect: bool = False  # every chosen region agreed completely


def choose(regions: list[Region], given: Frame, truth: Mesh, op: str, depth: float | None) -> Choice | None:
    """Pick plane, side and regions so the extrusion agrees with the finished part.

    ``depth`` is the blind length, or None for through all.
    """
    if not regions:
        return None
    depths = [depth * 0.5, depth * 0.2, depth * 0.8] if depth else [0.3, 1.0, 2.5]
    best: Choice | None = None
    frames = candidate_frames(given)
    for fr_index, fr in enumerate(frames):
        if fr_index == 1 and best is not None and _clean(best, regions, op):
            break  # cadmpeg's own plane explains the part: no need to look further
        for direction in (1, -1):
            chosen: list[Region] = []
            total = 0.0
            perfect = True
            for reg in regions:
                tests = [fr.point(q, direction * d) for q in reg.samples for d in depths[:2]]
                if op == "cut":
                    hollow = sum(1 for p in tests if not truth.inside(p)) / len(tests)
                    ring = [fr.point(q, direction * depths[0]) for q in reg.outer.outside_ring(0.4)]
                    walls = sum(1 for p in ring if truth.inside(p)) / len(ring) if ring else 0.0
                    ok = hollow >= 0.9 and walls >= 0.6
                    score = hollow + walls
                else:
                    solid = sum(1 for p in tests if truth.inside(p)) / len(tests)
                    ok = solid >= 0.9
                    score = solid
                if ok:
                    chosen.append(reg)
                    total += score
                    if score < (2.0 if op == "cut" else 1.0) - 1e-9:
                        perfect = False
            if not chosen:
                continue
            # prefer the given frame on ties: cadmpeg is right when it resolves the plane
            total += 0.01 if fr_index == 0 else 0.0
            if best is None or total > best.score + 1e-9:
                best = Choice(fr, direction, chosen, total, perfect)
    return best


def _clean(choice: Choice, regions: list[Region], op: str) -> bool:
    """Every chosen region agrees fully: nothing better is to be found."""
    return choice.perfect

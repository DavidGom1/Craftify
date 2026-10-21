"""Split a multi-body STEP file into files that keep only some of its solids."""

from __future__ import annotations

import re
from dataclasses import dataclass

_REF = re.compile(r"#(\d+)")
_HEAD = re.compile(r"\s*#(\d+)\s*=\s*(\(?\s*[A-Z_0-9]+)")
_SOLIDS = ("MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS", "SHELL_BASED_SURFACE_MODEL")  # every body, surfaces too


@dataclass
class Solid:
    id: int
    box: tuple[tuple[float, float, float], tuple[float, float, float]] | None  # from its vertices


class StepFile:
    def __init__(self, text: str):
        self.text = text
        self.statements = re.split(r"(?<=;)", text)
        self.line_of: dict[int, int] = {}
        self.kind: dict[int, str] = {}
        self.refs: dict[int, list[int]] = {}
        for k, st in enumerate(self.statements):
            m = _HEAD.match(st)
            if not m:
                continue
            eid = int(m.group(1))
            self.line_of[eid] = k
            self.kind[eid] = m.group(2).lstrip("( ").strip()
            body = st[st.index("=") + 1:]
            self.refs[eid] = [int(x) for x in _REF.findall(body)]

    def solids(self) -> list[Solid]:
        out = []
        for eid in sorted(self.kind):
            if self.kind[eid] in _SOLIDS:
                out.append(Solid(eid, self._vertex_box(eid)))
        return out

    def _reach(self, start: list[int], blocked: set[int]) -> set[int]:
        seen: set[int] = set()
        stack = [e for e in start if e not in blocked]
        while stack:
            e = stack.pop()
            if e in seen or e in blocked:
                continue
            seen.add(e)
            stack.extend(r for r in self.refs.get(e, ()) if r not in seen and r not in blocked)
        return seen

    def _vertex_box(self, solid: int):
        pts = []
        for e in self._reach([solid], set()):
            if self.kind.get(e) == "VERTEX_POINT":
                refs = self.refs.get(e) or []
                if refs:
                    p = self._point(refs[0])
                    if p:
                        pts.append(p)
        if not pts:
            return None
        return (tuple(min(p[k] for p in pts) for k in range(3)), tuple(max(p[k] for p in pts) for k in range(3)))

    def _point(self, eid: int):
        st = self.statements[self.line_of[eid]]
        m = re.search(r"\(\s*([-+0-9.eE]+)\s*,\s*([-+0-9.eE]+)\s*,\s*([-+0-9.eE]+)\s*\)", st)
        return tuple(float(m.group(k)) for k in (1, 2, 3)) if m else None

    def keep_only(self, keep: set[int]) -> str:
        """The file with every other solid (and what only it used) taken out."""
        drop = {s.id for s in self.solids()} - keep
        referenced = {r for refs in self.refs.values() for r in refs}
        roots = [e for e in self.refs if e not in referenced]
        statements = list(self.statements)
        dead: set[int] = set()
        for eid, refs in self.refs.items():
            hit = drop.intersection(refs)
            if not hit or eid in drop:
                continue
            st = statements[self.line_of[eid]]
            if self.kind[eid] in ("STYLED_ITEM", "OVER_RIDING_STYLED_ITEM") or "SHAPE_REPRESENTATION" not in self.kind[eid] and "REPRESENTATION" not in self.kind[eid]:
                # a style or other single-target link to a removed solid goes with it
                if any(f"#{d}" in st.split("(", 1)[1] for d in hit) and self.kind[eid] in ("STYLED_ITEM", "OVER_RIDING_STYLED_ITEM", "PRESENTATION_LAYER_ASSIGNMENT"):
                    dead.add(eid)
                    continue
            for d in hit:  # remove the solid from item lists
                st = re.sub(rf"#{d}\s*,\s*", "", st)
                st = re.sub(rf",\s*#{d}(?=\s*[),])", "", st)
            statements[self.line_of[eid]] = st
        live = self._reach([r for r in roots if r not in dead], drop | dead)
        out = []
        for k, st in enumerate(statements):
            m = _HEAD.match(st)
            if m and int(m.group(1)) not in live:
                continue
            out.append(st)
        return "".join(out)


def box_distance(a, b) -> float:
    """How far apart two boxes are (largest corner difference, mm)."""
    return max(max(abs(a[0][k] - b[0][k]), abs(a[1][k] - b[1][k])) for k in range(3))

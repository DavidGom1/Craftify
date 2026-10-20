"""A SolidWorks part as cadmpeg describes it (``cadmpeg dump``: CADIR JSON)."""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from ..mesh import Mesh
from ..solidcraft.script import NO_WINDOW

# Feature-tree entries that are not modelling steps.
_NOT_STEPS = {"tree_node", "datum_principal_plane"}


@dataclass
class PartIR:
    path: Path
    model: dict[str, Any]
    features: list[dict] = field(default_factory=list)  # modelling steps, in tree order
    sketches: dict[str, dict] = field(default_factory=dict)
    entities: dict[str, list[dict]] = field(default_factory=dict)  # by sketch id
    parameters: dict[str, list[dict]] = field(default_factory=dict)  # by owner feature id
    feature_by_id: dict[str, dict] = field(default_factory=dict)

    def mesh(self) -> Mesh:
        """SolidWorks' own display mesh of the part (outward normals)."""
        out = Mesh()
        for t in self.model.get("tessellations") or []:
            m = t.get("mesh") or {}
            if m.get("kind") != "shaded_strips":
                continue
            for strip in m.get("strips") or []:
                pts = [(v["position"]["x"], v["position"]["y"], v["position"]["z"]) for v in strip]
                nrm = [(v["normal"]["x"], v["normal"]["y"], v["normal"]["z"]) if v.get("normal") else None for v in strip]
                for k in range(len(pts) - 2):
                    ns = [n for n in nrm[k : k + 3] if n is not None]
                    outward = None
                    if ns:
                        outward = (sum(n[0] for n in ns), sum(n[1] for n in ns), sum(n[2] for n in ns))
                    out.add(pts[k], pts[k + 1], pts[k + 2], outward)
        return out


def load(path: Path, cadmpeg: Path, cache: Path | None = None) -> PartIR:
    if cache is not None and cache.is_file():
        model = json.loads(cache.read_text(encoding="utf-8"))
    else:
        proc = subprocess.run(
            [str(cadmpeg), "dump", str(path)], capture_output=True, text=True, encoding="utf-8",
            errors="replace", timeout=900, creationflags=NO_WINDOW,
        )
        if proc.returncode != 0:
            lines = (proc.stderr or "").strip().splitlines()
            raise RuntimeError(lines[-1] if lines else f"cadmpeg dump exited with {proc.returncode}")
        model = json.loads(proc.stdout)
        if cache is not None:
            cache.parent.mkdir(parents=True, exist_ok=True)
            cache.write_text(proc.stdout, encoding="utf-8")
    model = model.get("model", model)
    ir = PartIR(path=path, model=model)
    feats = sorted(model.get("features") or [], key=lambda f: f.get("ordinal", 0))
    ir.feature_by_id = {f["id"]: f for f in feats}
    ir.features = [f for f in feats if (f.get("definition") or {}).get("definition") not in _NOT_STEPS]
    ir.sketches = {s["id"]: s for s in model.get("sketches") or []}
    for e in model.get("sketch_entities") or []:
        ir.entities.setdefault(e.get("sketch"), []).append(e)
    for p in model.get("parameters") or []:
        ir.parameters.setdefault(p.get("owner"), []).append(p)
    return ir

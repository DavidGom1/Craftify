"""SolidCraft assembly designs from part designs and their placements (any source CAD)."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from . import script as sc
from .transform import Matrix, axis_angle

Vec = tuple[float, float, float]
IDENTITY: Matrix = (1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0)


@dataclass
class Placement:
    stem: str  # the part design's name (component name in SolidCraft)
    design: Path
    rotation: Matrix  # for column vectors, in SolidCraft's world
    translation_mm: Vec
    label: str = ""  # the source CAD's name for this instance


def assembly_script(name: str, placements: list[Placement], out_design: Path, metadata: dict) -> tuple[sc.Script, dict[str, str]]:
    """A script that links every part design once and places one occurrence per placement.

    A copy takes the current placement of the occurrence it copies, so every occurrence is made
    while the first one is still at the origin, and only then are they moved.
    """
    s = sc.Script()
    s.add("file.new", name=name)
    counts: dict[str, int] = {}
    todo: list[tuple[Placement, str]] = []
    mapping: dict[str, str] = {}
    for p in placements:
        n = counts.get(p.stem, 0) + 1
        counts[p.stem] = n
        if n == 1:
            s.add("component.insert_design", path=str(p.design), name=p.stem)
        else:
            s.add("occurrence.copy", occurrence=f"{p.stem}:1")
        occ = f"{p.stem}:{n}"
        todo.append((p, occ))
        mapping[p.label or occ] = occ
    for p, occ in todo:
        axis, angle = axis_angle(p.rotation)
        if angle > 1e-12:
            s.add("occurrence.move", occurrence=occ, space="world", axis=list(axis), angle=angle, origin=[0, 0, 0])
        if any(abs(x) > 1e-9 for x in p.translation_mm):
            s.add("occurrence.move", occurrence=occ, space="world", translate=list(p.translation_mm))
    s.add("document.metadata.set", key=sc.METADATA_KEY, value={**metadata, "instances": mapping})
    s.add("file.save_as", path=str(out_design))
    return s, mapping

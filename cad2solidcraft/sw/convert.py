"""SolidWorks assembly -> SolidCraft designs (phase 1: structure and geometry).

Every part becomes its own .solidcraft design (an Import feature holding the part's exact
geometry), and the assembly becomes a design whose components link to those part designs,
one occurrence per SolidWorks instance, placed where SolidWorks places it.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Callable

from .. import step_fix
from ..solidcraft import script as sc
from ..solidcraft.transform import Matrix, axis_angle, transpose
from .assembly import Assembly, Instance, read_assembly
from .container import read_file

Log = Callable[[str], None]

# SolidWorks stores a placement for row vectors (p' = p R + T); SolidCraft rotates column
# vectors, so the stored matrix is used transposed.
ROW_VECTOR_CONVENTION = True
# +90 degrees about X: the SolidWorks "up" (+Y) becomes SolidCraft's "up" (+Z).
Y_UP_TO_Z_UP: Matrix = (1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0)


@dataclass
class PartResult:
    stem: str
    source: str | None
    design: str | None = None
    step: str | None = None
    ok: bool = False
    volume_mm3: float | None = None
    bodies: int | None = None
    faces_fixed: int = 0
    circle_edges_fixed: int = 0
    seconds: float = 0.0
    warnings: list[str] = field(default_factory=list)


@dataclass
class Report:
    source: str
    output: str | None = None
    instances: int = 0
    placed: int = 0
    parts: list[PartResult] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    seconds: float = 0.0

    def ok(self) -> bool:
        return self.output is not None and all(p.ok for p in self.parts) and self.placed == self.instances


def find_cadmpeg(explicit: str | None = None) -> Path:
    for c in (explicit, shutil.which("cadmpeg"), str(Path.home() / ".cargo" / "bin" / "cadmpeg.exe")):
        if c and Path(c).is_file():
            return Path(c)
    raise FileNotFoundError("cadmpeg not found: install it with `cargo install cadmpeg`")


def resolve_reference(asm: Assembly, stem: str, search: list[Path]) -> Path | None:
    """Find the file an instance refers to, as SolidWorks does: by its file name."""
    wanted = {f"{stem}.sldprt".lower(), f"{stem}.sldasm".lower()}
    for folder in search:
        if folder.is_dir():
            for p in folder.iterdir():
                if p.name.lower() in wanted:
                    return p
    for ref in asm.references:  # the absolute paths the assembly recorded
        p = Path(ref)
        if p.name.lower() in wanted and p.is_file():
            return p
    for folder in search:  # last resort: anywhere below the search folders
        for p in folder.rglob("*"):
            if p.name.lower() in wanted:
                return p
    return None


def _run_cadmpeg(cadmpeg: Path, src: Path, step: Path) -> list[str]:
    proc = subprocess.run(
        [str(cadmpeg), "convert", str(src), "-o", str(step), "--force"],
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=900,
    )
    notes = [ln for ln in (proc.stdout + proc.stderr).splitlines() if ln.strip()]
    if proc.returncode != 0 or not step.is_file():
        raise RuntimeError(notes[-1] if notes else f"cadmpeg exited with {proc.returncode}")
    # Keep what affects the exact geometry; display tessellation and design-intent notes
    # (sketch relations, used later for the timeline) are not problems of this phase.
    keep = []
    for n in notes:
        low = n.lower()
        if "[warning/geometry]" in low and "tessellation(s) require" not in low:
            keep.append(n.strip())
        elif low.startswith("check:") and "0 error(s)" not in low:
            keep.append(n.strip())
    return keep


def step_string(text: str) -> str:
    """Text as the inside of a STEP (ISO 10303-21) string literal."""
    out = []
    for ch in text:
        if ch == "'":
            out.append("''")
        elif ch == "\\":
            out.append("\\\\")
        elif 32 <= ord(ch) < 127:
            out.append(ch)
        else:
            units = ch.encode("utf-16-be")
            out.append("\\X2\\" + units.hex().upper() + "\\X0\\")
    return "".join(out)


def name_step_product(step: Path, name: str) -> None:
    """Give the STEP's product (and so the imported body) the part's name."""
    text = step.read_text(encoding="latin-1")
    named = step_string(name)
    text = text.replace("PRODUCT('cadmpeg_model','cadmpeg_model'", f"PRODUCT('{named}','{named}'", 1)
    text = text.replace("FILE_NAME('cadmpeg_model'", f"FILE_NAME('{named}'", 1)
    step.write_text(text, encoding="latin-1")


def convert_part(src: Path, stem: str, out: Path, cli: Path, cadmpeg: Path, log: Log, up_y: bool = True) -> PartResult:
    t0 = time.monotonic()
    res = PartResult(stem=stem, source=str(src))
    build = out / "build"
    step = build / f"{stem}.step"
    design = out / "parts" / f"{stem}.solidcraft"
    design.parent.mkdir(parents=True, exist_ok=True)
    build.mkdir(parents=True, exist_ok=True)
    try:
        res.warnings += _run_cadmpeg(cadmpeg, src, step)
        name_step_product(step, stem)
        fix = step_fix.repair_file(step)
        res.faces_fixed = fix.flipped
        res.circle_edges_fixed = fix.circle_edges
        if fix.unknown:
            res.warnings.append(f"{fix.unknown} de {fix.faces} caras con orientación sin verificar")
        res.step = str(step)
        s = sc.Script()
        s.add("file.new", name=stem)
        s.add("file.insert_step", path=str(step), up="y" if up_y else "z")
        s.add("document.metadata.set", key=sc.METADATA_KEY, value={"source": str(src), "source_cad": "SolidWorks"})
        s.add("file.save_as", path=str(design))
        sc.run(cli, s.write(build / f"{stem}.part.json"))
        measured = sc.evaluate(cli, design)
        res.design = str(design)
        res.bodies = measured.get("body_count")
        res.volume_mm3 = (measured.get("total") or {}).get("volume_mm3")
        res.ok = bool(res.bodies)
        if not res.ok:
            res.warnings.append("la pieza convertida no tiene sólidos")
        for body in measured.get("bodies", []):
            if (body.get("volume_mm3") or 0) < 0:
                res.warnings.append(f"el sólido {body.get('name')} está del revés (volumen negativo)")
    except Exception as e:  # one bad part must not stop the assembly
        res.warnings.append(str(e))
    res.seconds = round(time.monotonic() - t0, 1)
    detail = " - " + "; ".join(res.warnings) if res.warnings else ""
    log(f"  {'OK ' if res.ok else 'ERR'} {stem} ({res.seconds} s){detail}")
    return res


def _matmul(a: Matrix, b: Matrix) -> Matrix:
    return tuple(sum(a[3 * r + k] * b[3 * k + c] for k in range(3)) for r in range(3) for c in range(3))


def _apply(m: Matrix, t: tuple[float, float, float]) -> tuple[float, float, float]:
    return (
        m[0] * t[0] + m[1] * t[1] + m[2] * t[2],
        m[3] * t[0] + m[4] * t[1] + m[5] * t[2],
        m[6] * t[0] + m[7] * t[1] + m[8] * t[2],
    )


def placement(inst: Instance, up_y: bool) -> tuple[Matrix, tuple[float, float, float]]:
    """Rotation (for column vectors) and translation (mm) of an occurrence in SolidCraft.

    With ``up_y`` every part design already holds its bodies turned upright (U), so an
    occurrence placed at M, t in SolidWorks becomes U·M·Uᵀ, U·t.
    """
    m = transpose(inst.rotation) if ROW_VECTOR_CONVENTION else inst.rotation
    t = inst.translation_mm
    if up_y:
        u = Y_UP_TO_Z_UP
        m = _matmul(_matmul(u, m), transpose(u))
        t = _apply(u, t)
    return m, t


def assembly_script(asm: Assembly, part_designs: dict[str, Path], out_design: Path, up_y: bool):
    s = sc.Script()
    s.add("file.new", name=asm.name)
    counts: dict[str, int] = {}
    mapping: dict[str, str] = {}
    placed = 0
    # Every occurrence first: a copy takes the current placement of the occurrence it copies,
    # so copies are made while the first occurrence is still at the origin.
    todo: list[tuple[Instance, str]] = []
    for inst in asm.instances:
        design = part_designs.get(inst.stem)
        if design is None:
            continue
        n = counts.get(inst.stem, 0) + 1
        counts[inst.stem] = n
        if n == 1:
            s.add("component.insert_design", path=str(design), name=inst.stem)
        else:
            s.add("occurrence.copy", occurrence=f"{inst.stem}:1")
        occ = f"{inst.stem}:{n}"
        mapping[inst.name] = occ
        todo.append((inst, occ))
    for inst, occ in todo:
        m, t = placement(inst, up_y)
        axis, angle = axis_angle(m)
        if angle > 1e-12:
            s.add("occurrence.move", occurrence=occ, space="world", axis=list(axis), angle=angle, origin=[0, 0, 0])
        if any(abs(x) > 1e-9 for x in t):
            s.add("occurrence.move", occurrence=occ, space="world", translate=list(t))
        placed += 1
    meta = {"source": str(asm.path), "source_cad": "SolidWorks", "instances": mapping, "up": "y" if up_y else "z"}
    s.add("document.metadata.set", key=sc.METADATA_KEY, value=meta)
    s.add("file.save_as", path=str(out_design))
    return s, placed, mapping


PREVIEW_SOURCE = "preview_solidworks.png"
PREVIEW_RESULT = "preview_solidcraft.png"


def convert_assembly(src: Path, out_root: Path, up_y: bool = True, cli: str | None = None,
                     cadmpeg: str | None = None, log: Log = print, picture: bool = True) -> Report:
    t0 = time.monotonic()
    cli_path = sc.find_cli(cli)
    cad_path = find_cadmpeg(cadmpeg)
    asm = read_assembly(src)
    out = out_root / asm.name
    out.mkdir(parents=True, exist_ok=True)
    report = Report(source=str(src), instances=len(asm.instances), warnings=list(asm.warnings))
    log(f"{asm.name}: {len(asm.instances)} instancias")

    part_designs: dict[str, Path] = {}
    for stem in dict.fromkeys(i.stem for i in asm.instances):
        ref = resolve_reference(asm, stem, [src.parent])
        if ref is None:
            report.parts.append(PartResult(stem=stem, source=None, warnings=["no se encuentra el fichero"]))
            log(f"  ERR {stem}: no se encuentra el fichero")
            continue
        if ref.suffix.lower() == ".sldasm":
            report.parts.append(PartResult(stem=stem, source=str(ref), warnings=["los subensamblajes aún no se convierten"]))
            log(f"  ERR {stem}: los subensamblajes aún no se convierten")
            continue
        r = convert_part(ref, stem, out, cli_path, cad_path, log, up_y)
        report.parts.append(r)
        if r.ok and r.design:
            part_designs[stem] = Path(r.design)

    design = out / f"{asm.name}.solidcraft"
    s, placed, _mapping = assembly_script(asm, part_designs, design, up_y)
    try:
        sc.run(cli_path, s.write(out / "build" / f"{asm.name}.assembly.json"))
        report.output = str(design)
        report.placed = placed
    except Exception as e:
        report.warnings.append(f"ensamblaje: {e}")
    _save_preview(src, out / PREVIEW_SOURCE)
    if picture and report.output:
        log("Dibujando una imagen del resultado…")
        try:
            sc.snapshot(cli_path, design, out / PREVIEW_RESULT, view="iso", width=800, height=600)
        except Exception as e:
            report.warnings.append(f"imagen: {e}")
    report.seconds = round(time.monotonic() - t0, 1)
    (out / "report.json").write_text(json.dumps(asdict(report), indent=1, ensure_ascii=False), encoding="utf-8")
    log(f"{'Terminado' if report.ok() else 'Terminado con avisos'}: {report.output} ({report.seconds} s)")
    return report


def _save_preview(src: Path, png: Path) -> None:
    for b in reversed(read_file(src)):
        if b.name == "PreviewPNG" and b.data and b.data.startswith(b"\x89PNG"):
            png.write_bytes(b.data)
            return

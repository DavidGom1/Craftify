"""Fusion design (.f3d / .f3z) -> SolidCraft designs.

Fusion itself exports the design (see bridge.py): one STEP per component with bodies, and the
placement of every occurrence, external references included. Each component becomes a part
design; the assembly links them, one occurrence per Fusion occurrence. Each part is checked
against the exact volumes and boxes Fusion reports.
"""

from __future__ import annotations

import io
import json
import re
import shutil
import time
import unicodedata
import zipfile
from dataclasses import asdict
from pathlib import Path

from .. import step_fix
from ..solidcraft import script as sc
from ..solidcraft.assembly import IDENTITY, Placement, assembly_script
from ..solidcraft.transform import Matrix, transpose
from ..sw.convert import (BOX_TOLERANCE, PREVIEW_RESULT, VOLUME_TOLERANCE, Y_UP_TO_Z_UP, Log, PartResult, Progress,
                          Report, _apply, _matmul)
from . import bridge

PREVIEW_FUSION = "preview_fusion.png"
MAX_PREVIEW = 20 * 1024 * 1024


def preview_png(src: Path) -> bytes | None:
    """The preview picture Fusion keeps inside a .f3d (or the root design of a .f3z)."""
    try:
        with zipfile.ZipFile(src) as z:
            names = z.namelist()
            if "Manifest.json" in names and not any(n.startswith("FusionAssetName") for n in names):
                root = json.loads(z.read("Manifest.json")).get("root")
                info = z.getinfo(root)
                if info.file_size > 512 * 1024 * 1024:
                    return None
                inner = zipfile.ZipFile(io.BytesIO(z.read(root)))
            else:
                inner = z
            pngs = [i for i in inner.infolist() if i.filename.lower().endswith(".png") and "preview" in i.filename.lower()
                    and i.file_size < MAX_PREVIEW]
            if not pngs:
                return None
            best = max(pngs, key=lambda i: i.file_size)
            data = inner.read(best)
            return data if data.startswith(b"\x89PNG") else None
    except Exception:
        return None


def _stem(name: str, used: set[str]) -> str:
    text = unicodedata.normalize("NFKC", name)
    text = re.sub(r'[<>:"/\\|?*\x00-\x1f]+', "_", text).strip(" .") or "Componente"
    stem, n = text, 2
    while stem.lower() in used:
        stem, n = f"{text} ({n})", n + 1
    used.add(stem.lower())
    return stem


def _turned_box(box: list[list[float]] | None, m: Matrix | None) -> tuple[list[float], list[float]] | None:
    if not box:
        return None
    lo, hi = box
    if m is None:
        return lo, hi
    corners = [(x, y, z) for x in (lo[0], hi[0]) for y in (lo[1], hi[1]) for z in (lo[2], hi[2])]
    pts = [_apply(m, c) for c in corners]
    return [min(p[k] for p in pts) for k in range(3)], [max(p[k] for p in pts) for k in range(3)]


def _convert_component(c: dict, job: Path, out: Path, stem: str, cli: Path, turn: bool, src: Path, log: Log) -> PartResult:
    t0 = time.monotonic()
    res = PartResult(stem=stem, source=str(src))
    build = out / "build"
    build.mkdir(parents=True, exist_ok=True)
    design = out / "parts" / f"{stem}.solidcraft"
    design.parent.mkdir(parents=True, exist_ok=True)
    step = build / f"{stem}.step"
    try:
        shutil.copyfile(job / c["step"], step)
        if turn:
            step.write_text(step_fix.rotate_text(step.read_text(encoding="latin-1"), Y_UP_TO_Z_UP), encoding="latin-1")
        res.step = str(step)
        s = sc.Script()
        s.add("file.new", name=stem)
        s.add("file.insert_step", path=str(step))
        s.add("document.metadata.set", key=sc.METADATA_KEY,
              value={"source": str(src), "source_cad": "Fusion", "component": c.get("name")})
        s.add("file.save_as", path=str(design))
        sc.run(cli, s.write(build / f"{stem}.part.json"))
        measured = sc.evaluate(cli, design)
        res.design = str(design)
        res.bodies = measured.get("body_count")
        res.volume_mm3 = (measured.get("total") or {}).get("volume_mm3")
        res.ok = bool(res.bodies)
        res.repair = "STEP de Fusion"
        res.source_volume_mm3 = c.get("volume_mm3")
        truth_box = _turned_box(c.get("bbox_mm"), Y_UP_TO_Z_UP if turn else None)
        boxes = [b.get("bbox") for b in measured.get("bodies") or [] if b.get("bbox")]
        if res.volume_mm3 is not None and res.source_volume_mm3 and truth_box and boxes:
            lo, hi = truth_box
            blo = [min(b["min"][k] for b in boxes) for k in range(3)]
            bhi = [max(b["max"][k] for b in boxes) for k in range(3)]
            box_err = max(max(abs(blo[k] - lo[k]), abs(bhi[k] - hi[k])) for k in range(3))
            vol_err = abs(res.volume_mm3 - res.source_volume_mm3) / max(abs(res.source_volume_mm3), 1e-9)
            res.matches_source = vol_err <= VOLUME_TOLERANCE and box_err <= BOX_TOLERANCE
            if not res.matches_source:
                res.warnings.append(f"no coincide con Fusion: volumen {res.volume_mm3 / 1000:.2f} cm³ frente a "
                                    f"{res.source_volume_mm3 / 1000:.2f} cm³, caja desviada {box_err:.2f} mm")
        if not res.ok:
            res.warnings.append("la pieza convertida no tiene sólidos")
    except Exception as e:
        res.warnings.append(str(e))
    res.seconds = round(time.monotonic() - t0, 1)
    detail = " - " + "; ".join(res.warnings) if res.warnings else ""
    log(f"  {'OK ' if res.ok else 'ERR'} {stem} ({res.seconds} s){detail}")
    return res


def convert_fusion(src: Path, out_root: Path, up_y: bool = True, cli: str | None = None, cadmpeg: str | None = None,
                   log: Log = print, picture: bool = True, progress: Progress | None = None) -> Report:
    def tell(stage: str, done: int, total: int, part: PartResult | None = None) -> None:
        if progress is not None:
            progress(stage, done, total, part)

    t0 = time.monotonic()
    cli_path = sc.find_cli(cli)
    out = out_root / src.stem
    out.mkdir(parents=True, exist_ok=True)
    report = Report(source=str(src))
    log(f"{src.stem}: diseño de Fusion")
    tell("export", 0, 0)
    data, job = bridge.export(src, log)
    turn = up_y and data.get("up", "y") == "y"
    comps = [c for c in data.get("components", []) if c.get("step")]
    used: set[str] = set()
    stems = {c["id"]: _stem(c.get("name") or "Componente", used) for c in comps}
    tell("start", 0, len(comps))
    designs: dict[str, Path] = {}
    for n, c in enumerate(comps, 1):
        r = _convert_component(c, job, out, stems[c["id"]], cli_path, turn, src, log)
        report.parts.append(r)
        if r.ok and r.design:
            designs[c["id"]] = Path(r.design)
        tell("part", n, len(comps), r)

    placements: list[Placement] = []
    if "root" in designs:
        placements.append(Placement(stems["root"], designs["root"], IDENTITY, (0.0, 0.0, 0.0), data.get("name", "root")))
    for o in data.get("occurrences", []):
        cid = o.get("component")
        if cid not in designs:
            continue
        m: Matrix = tuple(v for row in o["rotation"] for v in row)  # type: ignore[assignment]
        t = tuple(o["translation_mm"])
        if turn:
            m = _matmul(_matmul(Y_UP_TO_Z_UP, m), transpose(Y_UP_TO_Z_UP))
            t = _apply(Y_UP_TO_Z_UP, t)
        placements.append(Placement(stems[cid], designs[cid], m, t, o.get("path", "")))
    report.instances = len(placements)
    tell("assembly", len(comps), len(comps))

    if len(placements) == 1 and placements[0].stem == stems.get("root") and not data.get("occurrences"):
        report.output = str(placements[0].design)  # a single part
        report.placed = 1
    elif placements:
        design = out / f"{src.stem}.solidcraft"
        s, _ = assembly_script(src.stem, placements, design, {"source": str(src), "source_cad": "Fusion",
                                                               "up": "y" if turn else "z"})
        try:
            sc.run(cli_path, s.write(out / "build" / f"{src.stem}.assembly.json"))
            report.output = str(design)
            report.placed = len(placements)
        except Exception as e:
            report.warnings.append(f"ensamblaje: {e}")
    else:
        report.warnings.append("el diseño no tiene sólidos")

    png = preview_png(src)
    if png:
        (out / PREVIEW_FUSION).write_bytes(png)
    if picture and report.output:
        tell("picture", len(comps), len(comps))
        log("Dibujando una imagen del resultado…")
        try:
            sc.snapshot(cli_path, Path(report.output), out / PREVIEW_RESULT, view="iso", width=800, height=600)
        except Exception as e:
            report.warnings.append(f"imagen: {e}")
    report.seconds = round(time.monotonic() - t0, 1)
    (out / "report.json").write_text(json.dumps(asdict(report), indent=1, ensure_ascii=False), encoding="utf-8")
    log(f"{'Terminado' if report.ok() else 'Terminado con avisos'}: {report.output} ({report.seconds} s)")
    return report

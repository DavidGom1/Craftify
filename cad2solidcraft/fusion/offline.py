"""Fusion design (.f3d / .f3z) -> SolidCraft designs, without Fusion installed.

* Structure: our own reader of the Design segment (f3d.py): components, every occurrence
  placement with its parent, which bodies each component owns, and each body's cached box.
* Geometry: cadmpeg converts each document's bodies to STEP. It writes them in a fixed order
  (by BREP blob, then by body within the blob), which tells which component each one belongs
  to. The file is split per component and tidied the way Fusion's own export writes STEP
  (step_clean.py).
* External references (.f3z): a component whose name is the root component of another document
  in the archive is that document, placed there. Its geometry and inner placements come from
  that document (the copies the referencing design keeps are not grouped by component).
Each part is checked against the boxes Fusion cached for its bodies.
"""

from __future__ import annotations

import json
import re
import subprocess
import time
import unicodedata
import zipfile
from dataclasses import asdict
from pathlib import Path

from ..solidcraft import script as sc
from ..solidcraft.assembly import IDENTITY, Placement, assembly_script
from ..step_clean import clean
from ..step_split import StepFile
from ..sw.convert import BOX_TOLERANCE, PREVIEW_RESULT, Log, PartResult, Progress, Report, find_cadmpeg
from .convert import PREVIEW_FUSION, preview_png
from .f3d import IDENTITY4, Assembly, F3dError, open_f3d, read_assembly

Mat = tuple[float, ...]  # row-major 4x4, column vectors, mm


def _mul(a: Mat, b: Mat) -> Mat:
    return tuple(sum(a[4 * r + k] * b[4 * k + c] for k in range(4)) for r in range(4) for c in range(4))


def _stem(name: str, used: set[str]) -> str:
    text = unicodedata.normalize("NFKC", name)
    text = re.sub(r'[<>:"/\\|?*\x00-\x1f]+', "_", text).strip(" .") or "Componente"
    stem, n = text, 2
    while stem.lower() in used:
        stem, n = f"{text} ({n})", n + 1
    used.add(stem.lower())
    return stem


class Document:
    """One .f3d (a whole file or a member of a .f3z)."""

    def __init__(self, key: str, data: bytes):
        self.key = key
        self.data = data
        f = open_f3d(data)
        self.asm: Assembly = read_assembly(f)
        designs = (f.redirections or {}).get("designs") or []
        mine = [d for d in designs if d.get("targetFileName") == key] or designs[:1]
        self.name = (mine[0].get("displayName") if mine else None) or Path(key).stem
        root = self.asm.components.get(self.asm.root) if self.asm.root is not None else None
        self.root_name = root.name if root else None
        self.xrefs: dict[int, Document] = {}  # component id -> the document it is

    def children(self, cid: int) -> list:
        return [p for p in self.asm.placements if p.parent == cid]


def _load(src: Path) -> tuple[Document, dict[str, Document]]:
    docs: dict[str, Document] = {}
    if src.suffix.lower() == ".f3z":
        with zipfile.ZipFile(src) as z:
            root_name = json.loads(z.read("Manifest.json")).get("root")
            for info in z.infolist():
                if info.filename.lower().endswith(".f3d") and "/" not in info.filename:
                    docs[info.filename] = Document(info.filename, z.read(info))
        if root_name not in docs:
            raise F3dError("the archive has no root design")
        root = docs[root_name]
    else:
        root = docs[src.name] = Document(src.name, src.read_bytes())
    by_root = {d.root_name: d for d in docs.values() if d.root_name}
    for d in docs.values():
        for cid, c in d.asm.components.items():
            other = by_root.get(c.name)
            if cid != d.asm.root and other is not None and other is not d:
                d.xrefs[cid] = other
    return root, docs


def _plan(root: Document) -> list[tuple[Document, int, str]]:
    """The parts to make: every placed component with bodies of its own, and its file name."""
    seen: dict[tuple[str, int], Document] = {}

    def need(d: Document, cid: int, _w: Mat, _p: str) -> None:
        if d.asm.components[cid].bodies:
            seen.setdefault((d.key, cid), d)

    _walk(root, need)
    used: set[str] = set()
    by_doc: dict[str, list[tuple[Document, int]]] = {}
    for (key, cid), d in seen.items():
        by_doc.setdefault(key, []).append((d, cid))
    return [(d, cid, _stem(d.asm.components[cid].name, used)) for parts in by_doc.values() for d, cid in sorted(parts, key=lambda x: x[1])]


def part_names(src: Path) -> list[str]:
    """The parts a conversion will make, in the order it makes them (for listing them first)."""
    root, _docs = _load(src)
    return [stem for _d, _c, stem in _plan(root)]


def _geometry(doc: Document, cadmpeg: Path, build: Path, log: Log) -> StepFile | None:
    source = build / "documents" / doc.key
    source.parent.mkdir(parents=True, exist_ok=True)
    source.write_bytes(doc.data)
    step = build / f"{Path(doc.key).stem}.all.step"
    proc = subprocess.run([str(cadmpeg), "convert", str(source), "-o", str(step), "--force", "--allow-errors"],
                          capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=3600,
                          creationflags=sc.NO_WINDOW)
    if not step.is_file():
        lines = (proc.stdout + proc.stderr).strip().splitlines()
        log(f"  cadmpeg no ha podido leer {doc.name}: {lines[-1] if lines else proc.returncode}")
        return None
    return StepFile(step.read_text(encoding="latin-1"))


def _assign(doc: Document, step: StepFile, wanted: set[int]) -> dict[int, list[int]]:
    """Component id -> its bodies in the STEP file."""
    order = sorted(((b.brep, b.selector, cid, b.box) for cid, c in doc.asm.components.items() for b in c.bodies),
                   key=lambda x: (x[0], x[1]))
    items = step.solids()  # by entity id: the order cadmpeg wrote them
    out: dict[int, list[int]] = {}
    if len(order) == len(items):
        for (_brep, _sel, cid, _box), s in zip(order, items):
            if cid in wanted:
                out.setdefault(cid, []).append(s.id)
        return out
    # cadmpeg left some body out: each solid to the cached box that holds it best
    slots = [(cid, box) for _b, _s, cid, box in order if box and cid in wanted]
    pairs = []
    for s in items:
        if s.box is None:
            continue
        for i, (cid, box) in enumerate(slots):
            outside = sum(max(0.0, box[0][k] - s.box[0][k]) + max(0.0, s.box[1][k] - box[1][k]) for k in range(3))
            if outside <= 0.05 * 6:
                slack = sum((box[1][k] - box[0][k]) - (s.box[1][k] - s.box[0][k]) for k in range(3))
                pairs.append((outside * 1000 + slack, s.id, i))
    pairs.sort()
    taken_s, taken_b = set(), set()
    for _cost, sid, i in pairs:
        if sid not in taken_s and i not in taken_b:
            taken_s.add(sid)
            taken_b.add(i)
            out.setdefault(slots[i][0], []).append(sid)
    return out


def _box_error(cached, got) -> str | None:
    """Fusion caches a box that holds the body but may be a little loose round curved faces:
    the part must lie inside it and fill it to within half a percent of its size."""
    out = max(max(cached[0][k] - got[0][k], got[1][k] - cached[1][k]) for k in range(3))
    if out > BOX_TOLERANCE:
        return f"se sale {out:.2f} mm de la caja que guarda Fusion"
    size = max(cached[1][k] - cached[0][k] for k in range(3))
    short = max(max(got[0][k] - cached[0][k], cached[1][k] - got[1][k]) for k in range(3))
    if short > max(BOX_TOLERANCE, 0.005 * size):
        return f"le faltan {short:.2f} mm para llenar la caja que guarda Fusion"
    return None


def _import_warnings(results: list, step: StepFile) -> list[str]:
    """What SolidCraft could not read well (a surface body is open on purpose)."""
    def surface(w: str) -> bool:
        m = re.match(r"solid #(\d+): imported as an open body", w)
        return bool(m) and step.kind.get(int(m.group(1))) == "SHELL_BASED_SURFACE_MODEL"

    for r in results:
        if isinstance(r, dict) and "created_bodies" in r:
            return [w for w in r.get("warnings") or [] if "could not be meshed" not in w and not surface(w)]
    return []


def _part(doc: Document, cid: int, solids: list[int], step: StepFile, stem: str, out: Path, cli: Path,
          src: Path, log: Log) -> PartResult:
    t0 = time.monotonic()
    comp = doc.asm.components[cid]
    res = PartResult(stem=stem, source=str(src))
    build = out / "build"
    design = out / "parts" / f"{stem}.solidcraft"
    design.parent.mkdir(parents=True, exist_ok=True)
    try:
        part_step = build / f"{stem}.step"
        text, done = clean(step.keep_only(set(solids)))
        part_step.write_text(text, encoding="latin-1")
        res.step = str(part_step)
        res.repair = "lectura directa" + (f" ({done['seams']} costuras)" if done["seams"] else "")
        s = sc.Script()
        s.add("file.new", name=stem)
        s.add("file.insert_step", path=str(part_step))
        s.add("document.metadata.set", key=sc.METADATA_KEY,
              value={"source": str(src), "source_cad": "Fusion", "document": doc.name, "component": comp.name})
        s.add("file.save_as", path=str(design))
        results = sc.run(cli, s.write(build / f"{stem}.part.json"))
        problems = _import_warnings(results, step)
        if problems:
            res.warnings.append(f"SolidCraft avisa al leerla: {problems[0]}" + (f" (y {len(problems) - 1} más)" if len(problems) > 1 else ""))
        measured = sc.evaluate(cli, design)
        res.design = str(design)
        res.bodies = measured.get("body_count")
        res.volume_mm3 = (measured.get("total") or {}).get("volume_mm3")
        res.ok = bool(res.bodies)
        boxes = [b.get("bbox") for b in measured.get("bodies") or [] if b.get("bbox")]
        if comp.boxes and boxes:
            truth = (tuple(min(b[0][k] for b in comp.boxes) for k in range(3)),
                     tuple(max(b[1][k] for b in comp.boxes) for k in range(3)))
            got = (tuple(min(b["min"][k] for b in boxes) for k in range(3)),
                   tuple(max(b["max"][k] for b in boxes) for k in range(3)))
            err = _box_error(truth, got)
            res.matches_source = err is None and not problems
            if err:
                res.warnings.append(f"no coincide con Fusion: {err}")
        elif problems:
            res.matches_source = False
        if len(solids) < len(comp.bodies):
            res.warnings.append(f"{len(comp.bodies) - len(solids)} de {len(comp.bodies)} cuerpos no se han podido leer")
        for body in measured.get("bodies", []):
            if (body.get("volume_mm3") or 0) < 0:
                res.warnings.append(f"el sólido {body.get('name')} está del revés (volumen negativo)")
        if not res.ok:
            res.warnings.append("la pieza convertida no tiene sólidos")
    except Exception as e:
        res.warnings.append(str(e))
    res.seconds = round(time.monotonic() - t0, 1)
    detail = " - " + "; ".join(res.warnings) if res.warnings else ""
    log(f"  {'OK ' if res.ok else 'ERR'} {stem} ({res.seconds} s){detail}")
    return res


def _walk(root: Document, visit) -> None:
    """Every placed component, document by document: visit(doc, cid, world matrix, path)."""
    def go(d: Document, cid: int, world: Mat, path: str, depth: int) -> None:
        visit(d, cid, world, path)
        if depth > 16:
            return
        for p in d.children(cid):
            child = _mul(world, p.matrix)
            name = d.asm.components[p.target].name if p.target in d.asm.components else "?"
            if p.target in d.xrefs:
                other = d.xrefs[p.target]
                go(other, other.asm.root, child, f"{path}/{name}", depth + 1)
            elif p.target in d.asm.components:
                go(d, p.target, child, f"{path}/{name}", depth)

    if root.asm.root is not None:
        go(root, root.asm.root, IDENTITY4, root.name, 0)


def convert_fusion_offline(src: Path, out_root: Path, up_y: bool = False, cli: str | None = None,
                           cadmpeg: str | None = None, log: Log = print, picture: bool = True,
                           progress: Progress | None = None) -> Report:
    def tell(stage: str, done: int, total: int, part: PartResult | None = None) -> None:
        if progress is not None:
            progress(stage, done, total, part)

    t0 = time.monotonic()
    cli_path = sc.find_cli(cli)
    cad_path = find_cadmpeg(cadmpeg)
    out = out_root / src.stem
    build = out / "build"
    build.mkdir(parents=True, exist_ok=True)
    report = Report(source=str(src))
    log(f"{src.stem}: diseño de Fusion")
    root, docs = _load(src)
    log(f"  {len(docs)} documento(s): " + ", ".join(d.name for d in docs.values()))

    plan = _plan(root)
    tell("start", 0, len(plan))
    designs: dict[tuple[str, int], Path] = {}
    stems = {(d.key, cid): stem for d, cid, stem in plan}
    steps: dict[str, StepFile | None] = {}
    assigned: dict[str, dict[int, list[int]]] = {}
    for n, (d, cid, stem) in enumerate(plan, 1):
        if d.key not in steps:  # each document's geometry once, when its first part comes
            steps[d.key] = _geometry(d, cad_path, build, log)
            wanted_here = {c for dd, c, _s in plan if dd is d}
            assigned[d.key] = _assign(d, steps[d.key], wanted_here) if steps[d.key] else {}
        step = steps[d.key]
        solids = assigned[d.key].get(cid, [])
        if step is None or not solids:
            r = PartResult(stem=stem, source=str(src), warnings=["no se ha podido leer su geometría"])
            log(f"  ERR {stem}: no se ha podido leer su geometría")
        else:
            r = _part(d, cid, solids, step, stem, out, cli_path, src, log)
            if r.ok and r.design:
                designs[(d.key, cid)] = Path(r.design)
        report.parts.append(r)
        tell("part", n, len(plan), r)
    total = len(plan)
    needed = {(d.key, cid) for d, cid, _s in plan}

    placements: list[Placement] = []
    wanted = 0

    def place(d: Document, cid: int, w: Mat, path: str) -> None:
        nonlocal wanted
        key = (d.key, cid)
        if key in needed:
            wanted += 1
        if key in designs:
            m = (w[0], w[1], w[2], w[4], w[5], w[6], w[8], w[9], w[10])
            placements.append(Placement(stems[key], designs[key], m, (w[3], w[7], w[11]), path))

    _walk(root, place)
    report.instances = wanted
    tell("assembly", total, total)
    lone = placements[0] if len(placements) == 1 else None
    if wanted == 1 and lone and lone.rotation == IDENTITY and not any(lone.translation_mm):
        report.output = str(placements[0].design)  # a single part where the design has it
        report.placed = 1
    elif placements:
        design = out / f"{src.stem}.solidcraft"
        s, _ = assembly_script(src.stem, placements, design, {"source": str(src), "source_cad": "Fusion"})
        try:
            sc.run(cli_path, s.write(build / f"{src.stem}.assembly.json"))
            report.output = str(design)
            report.placed = len(placements)
        except Exception as e:
            report.warnings.append(f"ensamblaje: {e}")
    else:
        report.warnings.append("el diseño no tiene sólidos que se hayan podido leer")

    png = preview_png(src)
    if png:
        (out / PREVIEW_FUSION).write_bytes(png)
    if picture and report.output:
        tell("picture", total, total)
        log("Dibujando una imagen del resultado…")
        try:
            sc.snapshot(cli_path, Path(report.output), out / PREVIEW_RESULT, view="iso", width=800, height=600)
        except Exception as e:
            report.warnings.append(f"imagen: {e}")
    report.seconds = round(time.monotonic() - t0, 1)
    (out / "report.json").write_text(json.dumps(asdict(report), indent=1, ensure_ascii=False), encoding="utf-8")
    log(f"{'Terminado' if report.ok() else 'Terminado con avisos'}: {report.output} ({report.seconds} s)")
    return report

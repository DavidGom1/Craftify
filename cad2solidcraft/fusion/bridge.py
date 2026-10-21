"""Talk to Fusion through the Cad2SolidCraftExporter add-in.

Fusion must be installed on this computer and signed in. The add-in is copied into Fusion's
add-ins folder (it starts with Fusion), Fusion is started when it is not running, and a job
folder is handed over; the add-in answers with STEP files and result.json in that folder.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import time
import uuid
from pathlib import Path
from typing import Callable

ADDIN = "Cad2SolidCraftExporter"
ROOT_DIR = Path(os.environ.get("LOCALAPPDATA", str(Path.home()))) / "cad2solidcraft" / "fusion"
JOBS = ROOT_DIR / "jobs"
ALIVE = ROOT_DIR / "alive.json"
ADDINS_DIR = Path(os.environ.get("APPDATA", str(Path.home()))) / "Autodesk" / "Autodesk Fusion 360" / "API" / "AddIns"

Log = Callable[[str], None]


class FusionError(RuntimeError):
    pass


def _addin_source() -> Path:
    for base in (Path(getattr(sys, "_MEIPASS", "")), Path(__file__).resolve().parents[2]):
        p = base / "fusion_addin" / ADDIN
        if (p / f"{ADDIN}.py").is_file():
            return p
    raise FusionError("the Fusion add-in is missing from this installation")


def install_addin() -> bool:
    """Copy the add-in into Fusion's add-ins folder; True when it changed (Fusion must restart)."""
    src = _addin_source()
    dst = ADDINS_DIR / ADDIN
    changed = False
    dst.mkdir(parents=True, exist_ok=True)
    for f in src.iterdir():
        if f.is_file():
            target = dst / f.name
            if not target.is_file() or target.read_bytes() != f.read_bytes():
                shutil.copy2(f, target)
                changed = True
    return changed


def find_fusion() -> Path | None:
    base = Path(os.environ.get("LOCALAPPDATA", "")) / "Autodesk" / "webdeploy" / "production"
    if not base.is_dir():
        return None
    launchers = sorted(base.glob("*/FusionLauncher.exe"), key=lambda p: p.stat().st_mtime, reverse=True)
    return launchers[0] if launchers else None


def _fusion_running() -> bool:
    out = subprocess.run(["tasklist", "/FI", "IMAGENAME eq Fusion360.exe", "/NH"], capture_output=True, text=True,
                         creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    return "Fusion360.exe" in out.stdout


def _alive(max_age: float = 6.0) -> bool:
    try:
        data = json.loads(ALIVE.read_text(encoding="utf-8"))
        return time.time() - float(data.get("time", 0)) < max_age
    except Exception:
        return False


def ensure_ready(log: Log, timeout: float = 420) -> None:
    """Fusion running with the add-in loaded, or a clear error."""
    changed = install_addin()
    if _alive():
        if changed:
            log("El complemento de Fusion se ha actualizado: reinicia Fusion para usar la versión nueva.")
        return
    if _fusion_running():
        raise FusionError(
            "Fusion está abierto pero el complemento de exportación no está cargado. "
            "Cierra Fusion y vuelve a convertir (se abrirá solo con el complemento), "
            "o actívalo en Utilidades > Complementos > Cad2SolidCraftExporter.")
    exe = find_fusion()
    if exe is None:
        raise FusionError("Fusion no está instalado en este equipo: hace falta para convertir ficheros de Fusion.")
    log("Abriendo Fusion…")
    subprocess.Popen([str(exe)], creationflags=getattr(subprocess, "DETACHED_PROCESS", 0))
    t0 = time.time()
    while time.time() - t0 < timeout:
        if _alive():
            log(f"Fusion listo en {time.time() - t0:.0f} s.")
            return
        time.sleep(2)
    raise FusionError("Fusion no ha cargado el complemento de exportación a tiempo. Comprueba que has iniciado sesión.")


def export(src: Path, log: Log, timeout: float = 1800) -> tuple[dict, Path]:
    """Have Fusion export a design; returns (result.json, the folder with the STEP files)."""
    ensure_ready(log)
    job = JOBS / f"{time.strftime('%Y%m%d-%H%M%S')}-{uuid.uuid4().hex[:6]}"
    job.mkdir(parents=True, exist_ok=True)
    (job / "job.json.tmp").write_text(json.dumps({"source": str(src)}, ensure_ascii=False), encoding="utf-8")
    os.replace(job / "job.json.tmp", job / "job.json")
    log("Exportando desde Fusion…")
    t0 = time.time()
    result = job / "result.json"
    while not result.is_file():
        if time.time() - t0 > timeout:
            raise FusionError("Fusion no ha terminado la exportación a tiempo.")
        if not _alive(max_age=30):
            raise FusionError("Fusion se ha cerrado durante la exportación.")
        time.sleep(1)
    data = json.loads(result.read_text(encoding="utf-8"))
    if not data.get("ok"):
        raise FusionError("Fusion no ha podido exportar el diseño:\n" + str(data.get("error", "")).strip().splitlines()[-1])
    log(f"Exportado desde Fusion en {time.time() - t0:.0f} s: {len(data.get('components', []))} componentes, "
        f"{len(data.get('occurrences', []))} ocurrencias.")
    return data, job

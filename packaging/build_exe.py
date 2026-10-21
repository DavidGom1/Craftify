"""Build "Convertir a SolidCraft.exe" and, with --shortcut, put a button for it on the desktop.

    python packaging/build_exe.py [--shortcut]

The result is dist/Convertir a SolidCraft/ (one folder with the .exe). It carries Python, the
window and cadmpeg, so the computers it runs on need only SolidCraft installed.
Needs: pip install -r requirements.txt pyinstaller flet-desktop, and cadmpeg on this computer.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NAME = "Convertir a SolidCraft"


def find_cadmpeg() -> Path:
    sys.path.insert(0, str(ROOT))
    from cad2solidcraft.sw.convert import find_cadmpeg as find

    return find()


def build() -> Path:
    flet = shutil.which("flet") or str(Path(os.environ.get("APPDATA", "")) / "Python" / f"Python{sys.version_info.major}{sys.version_info.minor}" / "Scripts" / "flet.exe")
    icon = ROOT / "packaging" / "icon.ico"
    if not icon.is_file():
        subprocess.run([sys.executable, str(ROOT / "packaging" / "make_icon.py")], check=True)
    cmd = [
        flet, "pack", str(ROOT / "packaging" / "launcher.py"),
        "--name", NAME,
        "--icon", str(icon),
        "--onedir",
        "--distpath", str(ROOT / "dist"),
        "--add-binary", f"{find_cadmpeg()}{os.pathsep}.",
        "--add-data", f"{icon}{os.pathsep}.",
        "--add-data", f"{ROOT / 'fusion_addin'}{os.pathsep}fusion_addin",
        "--product-name", NAME,
        "--file-description", "Convierte ensamblajes y piezas de SolidWorks en diseños de SolidCraft",
        "--company-name", "Hidroconta",
        "--product-version", "0.2.0",
        "--file-version", "0.2.0.0",
        "--copyright", "Hidroconta",
        "-y",
        "--", f"--paths={ROOT}", f"--workpath={ROOT / 'build' / 'pyinstaller'}",
        f"--specpath={ROOT / 'build'}",
    ]
    subprocess.run(cmd, check=True, cwd=ROOT)
    exe = ROOT / "dist" / NAME / f"{NAME}.exe"
    if not exe.is_file():
        raise SystemExit(f"the build did not produce {exe}")
    return exe


def shortcut(exe: Path) -> Path:
    """A desktop shortcut (the "button") to the packaged app."""
    desktop = Path(subprocess.run(
        ["powershell", "-NoProfile", "-Command", "[Environment]::GetFolderPath('Desktop')"],
        capture_output=True, text=True, check=True).stdout.strip())
    link = desktop / f"{NAME}.lnk"
    ps = (
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:LNK); "
        "$s.TargetPath = $env:EXE; $s.WorkingDirectory = $env:DIR; $s.IconLocation = $env:EXE + ',0'; "
        "$s.Description = 'Convierte ensamblajes y piezas de SolidWorks en diseños de SolidCraft'; $s.Save()"
    )
    env = dict(os.environ, LNK=str(link), EXE=str(exe), DIR=str(exe.parent))
    subprocess.run(["powershell", "-NoProfile", "-Command", ps], check=True, env=env)
    return link


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--shortcut", action="store_true", help="also put a shortcut on the desktop")
    a = ap.parse_args()
    exe = build()
    print("built", exe)
    if a.shortcut:
        print("shortcut", shortcut(exe))


if __name__ == "__main__":
    main()

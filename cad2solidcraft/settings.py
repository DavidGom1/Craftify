"""Where things are on this computer, and the window's remembered choices."""

from __future__ import annotations

import json
import os
from pathlib import Path

DEFAULT_OUT = Path.home() / "Documents" / "SolidCraft convertidos"
SETTINGS = Path(os.environ.get("APPDATA", str(Path.home()))) / "cad2solidcraft" / "settings.json"


def find_solidcraft() -> Path | None:
    """The SolidCraft desktop app, for opening results."""
    for base in (os.environ.get("ProgramFiles"), os.environ.get("LOCALAPPDATA")):
        if base:
            for p in (Path(base) / "SolidCraft" / "solidcraft.exe", Path(base) / "Programs" / "SolidCraft" / "solidcraft.exe"):
                if p.is_file():
                    return p
    return None


def load() -> dict:
    try:
        return json.loads(SETTINGS.read_text(encoding="utf-8"))
    except Exception:
        return {}


def save(data: dict) -> None:
    try:
        SETTINGS.parent.mkdir(parents=True, exist_ok=True)
        SETTINGS.write_text(json.dumps(data, indent=1, ensure_ascii=False), encoding="utf-8")
    except Exception:
        pass

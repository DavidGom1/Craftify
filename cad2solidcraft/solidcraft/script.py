"""Build SolidCraft command scripts and run them with solidcraft-cli."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

METADATA_KEY = "com.hidroconta.cad2solidcraft"


class SolidCraftError(RuntimeError):
    pass


@dataclass
class Script:
    commands: list[dict[str, Any]] = field(default_factory=list)

    def add(self, command: str, **params: Any) -> "Script":
        self.commands.append({"command": command, "params": params})
        return self

    def write(self, path: Path) -> Path:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"commands": self.commands}, indent=1, ensure_ascii=False), encoding="utf-8")
        return path


def find_cli(explicit: str | None = None) -> Path:
    candidates = [explicit, os.environ.get("SOLIDCRAFT_CLI"), shutil.which("solidcraft-cli")]
    for base in (os.environ.get("ProgramFiles"), os.environ.get("LOCALAPPDATA")):
        if base:
            candidates.append(str(Path(base) / "SolidCraft" / "solidcraft-cli.exe"))
            candidates.append(str(Path(base) / "Programs" / "SolidCraft" / "solidcraft-cli.exe"))
    for c in candidates:
        if c and Path(c).is_file():
            return Path(c)
    raise SolidCraftError("solidcraft-cli not found: install SolidCraft or set SOLIDCRAFT_CLI")


def run(cli: Path, script_path: Path, timeout: float = 1800) -> list[Any]:
    """Run a script; return each command's result. Raises SolidCraftError on failure."""
    results_path = script_path.with_suffix(".results.json")
    if results_path.exists():
        results_path.unlink()
    proc = subprocess.run(
        [str(cli), "run", str(script_path), "--results", "--quiet", "--json-out", str(results_path)],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
    )
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout).strip().splitlines()
        raise SolidCraftError(detail[-1] if detail else f"solidcraft-cli exited with {proc.returncode}")
    if not results_path.exists():
        return []
    data = json.loads(results_path.read_text(encoding="utf-8"))
    return data if isinstance(data, list) else [data]


def evaluate(cli: Path, design: Path, timeout: float = 600) -> dict[str, Any]:
    """Measurements of a design (bodies, volume, area, bounding boxes)."""
    out = design.with_suffix(".eval.json")
    proc = subprocess.run(
        [str(cli), "eval", str(design), "--json-out", str(out)],
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout,
    )
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout).strip().splitlines()
        raise SolidCraftError(detail[-1] if detail else f"solidcraft-cli eval exited with {proc.returncode}")
    return json.loads(out.read_text(encoding="utf-8"))


def snapshot(cli: Path, design: Path, png: Path, view: str = "iso", width: int = 1200, height: int = 900) -> None:
    proc = subprocess.run(
        [str(cli), "snapshot", "--in", str(design), "--out", str(png), "--view", view,
         "--width", str(width), "--height", str(height), "--no-grid"],
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=600,
    )
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout).strip().splitlines()
        raise SolidCraftError(detail[-1] if detail else "solidcraft-cli snapshot failed")

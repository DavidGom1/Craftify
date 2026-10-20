"""Command line: python -m cad2solidcraft <assembly.SLDASM> [--out DIR] [--up y|z]."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from .sw.convert import convert_assembly


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="cad2solidcraft", description="Convert a SolidWorks assembly into SolidCraft designs.")
    p.add_argument("source", type=Path, help="the .SLDASM assembly to convert")
    p.add_argument("--out", type=Path, default=Path.cwd() / "out", help="output folder (default: ./out)")
    p.add_argument("--up", choices=("y", "z"), default="y", help="the source's up axis (SolidWorks: y)")
    p.add_argument("--solidcraft-cli", help="path of solidcraft-cli.exe")
    p.add_argument("--cadmpeg", help="path of cadmpeg.exe")
    a = p.parse_args(argv)
    if a.source.suffix.lower() != ".sldasm" or not a.source.is_file():
        p.error("source must be an existing .SLDASM file")
    report = convert_assembly(a.source, a.out, up_y=a.up == "y", cli=a.solidcraft_cli, cadmpeg=a.cadmpeg)
    return 0 if report.ok() else 1


if __name__ == "__main__":
    sys.exit(main())

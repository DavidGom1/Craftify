"""Command line: python -m cad2solidcraft <assembly.SLDASM | part.SLDPRT> [--out DIR] [--up y|z]."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from .converter import SUPPORTED, convert


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="cad2solidcraft",
                                description="Convert SolidWorks assemblies and parts, and Fusion designs, into SolidCraft designs.")
    p.add_argument("source", type=Path, help="the .SLDASM, .SLDPRT, .f3d or .f3z file to convert")
    p.add_argument("--out", type=Path, default=Path.cwd() / "out", help="output folder (default: ./out)")
    p.add_argument("--up", choices=("y", "z"), default="y", help="turn Y-up models upright (y, the default) or keep them as they are (z)")
    p.add_argument("--solidcraft-cli", help="path of solidcraft-cli.exe")
    p.add_argument("--cadmpeg", help="path of cadmpeg.exe")
    a = p.parse_args(argv)
    if a.source.suffix.lower() not in SUPPORTED or not a.source.is_file():
        p.error("source must be an existing .SLDASM, .SLDPRT, .f3d or .f3z file")
    report = convert(a.source, a.out, up_y=a.up == "y", cli=a.solidcraft_cli, cadmpeg=a.cadmpeg)
    return 0 if report.ok() else 1


if __name__ == "__main__":
    sys.exit(main())

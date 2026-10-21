"""One entry point for every source CAD, chosen by the file's extension."""

from __future__ import annotations

from pathlib import Path

from .sw.convert import Report

SOLIDWORKS = (".sldasm", ".sldprt")
FUSION = (".f3d", ".f3z")
SUPPORTED = SOLIDWORKS + FUSION


def kind(src: Path) -> str:
    ext = src.suffix.lower()
    if ext == ".sldasm":
        return "Ensamblaje de SolidWorks"
    if ext == ".sldprt":
        return "Pieza de SolidWorks"
    if ext == ".f3z":
        return "Proyecto de Fusion (con referencias)"
    if ext == ".f3d":
        return "Diseño de Fusion"
    return "Fichero"


def convert(src: Path, out_root: Path, **kwargs) -> Report:
    ext = src.suffix.lower()
    if ext in FUSION:
        from .fusion.convert import convert_fusion

        return convert_fusion(src, out_root, **kwargs)
    from .sw.convert import convert as convert_sw

    return convert_sw(src, out_root, **kwargs)


def preview_png(src: Path) -> bytes | None:
    """The preview picture the source CAD saved inside the file."""
    if src.suffix.lower() in FUSION:
        from .fusion.convert import preview_png as fusion_preview

        return fusion_preview(src)
    from .sw.convert import preview_png as sw_preview

    return sw_preview(src)

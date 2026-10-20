"""Read the component instances of a SolidWorks assembly (.SLDASM).

Observed layout of the ``Contents/Config-0`` block, from the user's own files:
strings are MFC Unicode strings (``FF FE FF``, a length, UTF-16LE characters). Each placed
component is a record that starts with its instance name (``<file stem>-<n>``). 145 bytes
after the name ends comes its placement: either 13 little-endian doubles (a 3x3 rotation,
the translation in metres and the scale) or, when the component is not rotated, only the
translation and the scale. The name of the referenced configuration follows the placement.
"""

from __future__ import annotations

import math
import re
import struct
from dataclasses import dataclass, field
from pathlib import Path

from .container import Block, read_file

PLACEMENT_GAP = 145
_FULL_TO_CONFIG = 196  # placement start -> configuration string, rotated components
_SHORT_TO_CONFIG = 124  # placement start -> configuration string, translation only
_INSTANCE = re.compile(r"^(?P<stem>.+)-(?P<n>\d+)$")
IDENTITY = (1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0)


@dataclass(frozen=True)
class CString:
    start: int  # offset of the FF FE FF prefix
    end: int  # offset just past the last character
    text: str


@dataclass
class Instance:
    name: str  # SolidWorks instance name, e.g. "FLA 2.10-3"
    stem: str  # the referenced file's name without extension, e.g. "FLA 2.10"
    rotation: tuple[float, ...]  # 3x3, row-major, as stored
    translation_mm: tuple[float, float, float]
    configuration: str | None
    rotated: bool


@dataclass
class Assembly:
    path: Path
    name: str
    instances: list[Instance] = field(default_factory=list)
    references: list[str] = field(default_factory=list)  # every file path the assembly names
    warnings: list[str] = field(default_factory=list)


def read_cstrings(buf: bytes) -> list[CString]:
    out: list[CString] = []
    pos = 0
    while True:
        i = buf.find(b"\xff\xfe\xff", pos)
        if i < 0 or i + 4 > len(buf):
            return out
        n = buf[i + 3]
        start = i + 4
        if n == 0xFF:  # long string: u16 length follows
            if i + 6 > len(buf):
                return out
            n = struct.unpack_from("<H", buf, i + 4)[0]
            start = i + 6
        end = start + 2 * n
        if n == 0 or end > len(buf):
            pos = i + 1
            continue
        try:
            text = buf[start:end].decode("utf-16le")
        except UnicodeDecodeError:
            pos = i + 1
            continue
        if all(ch.isprintable() for ch in text):
            out.append(CString(i, end, text))
            pos = end
        else:
            pos = i + 1


def _is_rotation(m: tuple[float, ...], tol: float = 1e-6) -> bool:
    rows = (m[0:3], m[3:6], m[6:9])
    if any(abs(sum(x * x for x in r) - 1.0) > tol for r in rows):
        return False
    if any(abs(sum(a * b for a, b in zip(rows[i], rows[j]))) > tol for i in range(3) for j in range(i + 1, 3)):
        return False
    det = (
        m[0] * (m[4] * m[8] - m[5] * m[7])
        - m[1] * (m[3] * m[8] - m[5] * m[6])
        + m[2] * (m[3] * m[7] - m[4] * m[6])
    )
    return abs(det - 1.0) < tol


def _string_at(strings_by_start: dict[int, CString], pos: int) -> CString | None:
    return strings_by_start.get(pos)


def _placement(buf: bytes, at: int, strings_by_start: dict[int, CString]):
    """Return (rotation, translation_m, configuration, rotated) or None."""
    if at + 13 * 8 <= len(buf):
        v = struct.unpack_from("<13d", buf, at)
        if all(math.isfinite(x) for x in v) and _is_rotation(v[:9]) and abs(v[12] - 1.0) < 1e-9:
            cfg = _string_at(strings_by_start, at + _FULL_TO_CONFIG)
            if cfg is not None:
                return v[:9], v[9:12], cfg.text, True
    if at + 4 * 8 <= len(buf):
        v = struct.unpack_from("<4d", buf, at)
        if all(math.isfinite(x) for x in v) and abs(v[3] - 1.0) < 1e-9 and all(abs(x) < 1e4 for x in v[:3]):
            cfg = _string_at(strings_by_start, at + _SHORT_TO_CONFIG)
            if cfg is not None:
                return IDENTITY, v[:3], cfg.text, False
    return None


def parse_instances(config_block: bytes) -> list[Instance]:
    strings = read_cstrings(config_block)
    by_start = {s.start: s for s in strings}
    found: list[Instance] = []
    seen: set[str] = set()
    for s in strings:
        m = _INSTANCE.match(s.text)
        if not m or s.text in seen:
            continue
        placed = _placement(config_block, s.end + PLACEMENT_GAP, by_start)
        if placed is None:
            continue
        rotation, t_m, configuration, rotated = placed
        seen.add(s.text)
        found.append(
            Instance(
                name=s.text,
                stem=m.group("stem"),
                rotation=tuple(rotation),
                translation_mm=tuple(x * 1000.0 for x in t_m),
                configuration=configuration,
                rotated=rotated,
            )
        )
    return found


def _block(blocks: list[Block], name: str) -> bytes | None:
    """The last decodable block with this name (earlier copies are stale)."""
    for b in reversed(blocks):
        if b.name == name and b.data is not None:
            return b.data
    return None


def read_assembly(path: str | Path) -> Assembly:
    path = Path(path)
    blocks = read_file(path)
    asm = Assembly(path=path, name=path.stem)
    config = _block(blocks, "Contents/Config-0")
    if config is None:
        asm.warnings.append("no readable Contents/Config-0 block: the component list could not be read")
        return asm
    asm.instances = parse_instances(config)
    header = _block(blocks, "Header2")
    if header is not None:
        asm.references = [s.text for s in read_cstrings(header) if s.text.lower().endswith((".sldprt", ".sldasm"))]
    return asm

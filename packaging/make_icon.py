"""Draw the app icon (packaging/icon.ico) with the standard library only.

A rounded indigo square with a white isometric cube, rendered with 4x4 supersampling and
stored as PNG images inside one .ico (16 to 256 px).
"""

from __future__ import annotations

import math
import struct
import zlib
from pathlib import Path

TOP = (92, 107, 192)  # indigo, lighter at the top
BOTTOM = (48, 63, 159)
WHITE = (255, 255, 255)


def _rounded_box(x: float, y: float, half: float, radius: float) -> float:
    """Signed distance to a square of half-size `half` with rounded corners (centre 0,0)."""
    qx, qy = abs(x) - (half - radius), abs(y) - (half - radius)
    outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
    return outside + min(max(qx, qy), 0.0) - radius


def _segment(px: float, py: float, a: tuple[float, float], b: tuple[float, float]) -> float:
    ax, ay = a
    bx, by = b
    dx, dy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
    return math.hypot(px - ax - t * dx, py - ay - t * dy)


def _cube_lines() -> list[tuple[tuple[float, float], tuple[float, float]]]:
    r = 0.42  # cube radius in units of the half-size
    v = [(r * math.cos(math.radians(90 + 60 * k)), -r * math.sin(math.radians(90 + 60 * k))) for k in range(6)]
    centre = (0.0, 0.0)
    lines = [(v[k], v[(k + 1) % 6]) for k in range(6)]
    lines += [(centre, v[0]), (centre, v[2]), (centre, v[4])]  # the three inner edges
    return lines


def render(size: int) -> bytes:
    """RGBA pixels of the icon at `size` px."""
    ss = 4
    lines = _cube_lines()
    stroke = 0.075 if size >= 32 else 0.11
    out = bytearray()
    for j in range(size):
        for i in range(size):
            bg = fg = 0
            for sj in range(ss):
                for si in range(ss):
                    x = ((i + (si + 0.5) / ss) / size) * 2 - 1
                    y = ((j + (sj + 0.5) / ss) / size) * 2 - 1
                    if _rounded_box(x, y, 0.94, 0.36) <= 0:
                        bg += 1
                        if min(_segment(x, y, a, b) for a, b in lines) <= stroke / 2:
                            fg += 1
            n = ss * ss
            t = j / max(size - 1, 1)
            base = tuple(round(TOP[k] * (1 - t) + BOTTOM[k] * t) for k in range(3))
            if bg == 0:
                out += bytes((0, 0, 0, 0))
                continue
            mix = fg / bg
            rgb = tuple(round(base[k] * (1 - mix) + WHITE[k] * mix) for k in range(3))
            out += bytes((*rgb, round(255 * bg / n)))
    return bytes(out)


def png(size: int, rgba: bytes) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    rows = b"".join(b"\x00" + rgba[r * size * 4:(r + 1) * size * 4] for r in range(size))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))


def ico(images: list[tuple[int, bytes]]) -> bytes:
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, blobs = b"", b""
    for size, data in images:
        entries += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    return header + entries + blobs


def main() -> None:
    here = Path(__file__).resolve().parent
    images = []
    for size in (16, 24, 32, 48, 64, 128, 256):
        images.append((size, png(size, render(size))))
    (here / "icon.ico").write_bytes(ico(images))
    (here / "icon.png").write_bytes(images[-1][1])
    print("written", here / "icon.ico")


if __name__ == "__main__":
    main()

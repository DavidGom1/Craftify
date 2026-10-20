"""Block reader for the SolidWorks 2015+ file container.

Observed layout (from the user's own files, for interoperability):
each block starts with the marker ``14 00 06 00 08 00`` followed by five little-endian
u32 values -- flags, checksum, compressed size, raw size, name length -- then the block
name (each byte with its two nibbles swapped) and the payload. A payload whose compressed
size differs from its raw size is raw DEFLATE.
"""

from __future__ import annotations

import struct
import zlib
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator

MARKER = b"\x14\x00\x06\x00\x08\x00"
_HEADER = struct.Struct("<5I")
MAX_NAME = 1024
MAX_RAW = 512 * 1024 * 1024


@dataclass(frozen=True)
class Block:
    name: str
    offset: int
    comp_size: int
    raw_size: int
    data: bytes | None  # None when the payload could not be decoded


def _swap_nibbles(raw: bytes) -> bytes:
    return bytes(((b << 4) & 0xF0) | (b >> 4) for b in raw)


def _plausible_name(name: bytes) -> bool:
    return all(32 <= b < 127 for b in name)


def read_blocks(data: bytes) -> Iterator[Block]:
    """Yield every block of a SolidWorks container, in file order."""
    pos = 0
    end = len(data)
    while True:
        i = data.find(MARKER, pos)
        if i < 0:
            return
        head = i + len(MARKER)
        if head + _HEADER.size > end:
            return
        _flags, _crc, comp, raw, name_len = _HEADER.unpack_from(data, head)
        name_start = head + _HEADER.size
        payload_start = name_start + name_len
        if not (0 < name_len <= MAX_NAME) or raw > MAX_RAW or payload_start + comp > end:
            pos = i + 1
            continue
        name_bytes = _swap_nibbles(data[name_start:payload_start])
        if not _plausible_name(name_bytes):
            pos = i + 1
            continue
        payload = data[payload_start : payload_start + comp]
        content: bytes | None
        used = comp
        if comp == raw:
            content = payload
        else:
            try:
                content = zlib.decompress(payload, -15)
            except zlib.error:
                # Some headers state sizes that do not match their payload (the raw size is then
                # exactly twice the compressed one). Decode the DEFLATE stream to its own end.
                content, used = _inflate_to_end(data, payload_start)
        yield Block(name_bytes.decode("ascii"), i, used, len(content) if content else raw, content)
        pos = payload_start + used


def _inflate_to_end(data: bytes, start: int) -> tuple[bytes | None, int]:
    """Inflate one raw DEFLATE stream starting at ``start``; (None, 0) if it is not one."""
    inflater = zlib.decompressobj(-15)
    window = data[start : start + MAX_RAW]
    try:
        out = inflater.decompress(window, MAX_RAW)
    except zlib.error:
        return None, 0
    if not inflater.eof:
        return None, 0
    return out, len(window) - len(inflater.unused_data)


def read_file(path: str | Path) -> list[Block]:
    return list(read_blocks(Path(path).read_bytes()))

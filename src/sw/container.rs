//! Block reader for the SolidWorks 2015+ file container. Port of `sw/container.py`.
//!
//! Observed layout (from the user's own files, for interoperability): each block starts with
//! the marker `14 00 06 00 08 00` followed by five little-endian u32 values -- flags, checksum,
//! compressed size, raw size, name length -- then the block name (each byte with its two
//! nibbles swapped) and the payload. A payload whose compressed size differs from its raw size
//! is raw DEFLATE.

use std::path::Path;

use anyhow::Result;
use flate2::{Decompress, FlushDecompress, Status};

const MARKER: &[u8] = b"\x14\x00\x06\x00\x08\x00";
const MAX_NAME: u32 = 1024;
const MAX_RAW: usize = 512 * 1024 * 1024;

pub struct Block {
    pub name: String,
    pub offset: usize,
    pub comp_size: usize,
    pub raw_size: usize,
    /// None when the payload could not be decoded.
    pub data: Option<Vec<u8>>,
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn find(data: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= data.len() {
        return None;
    }
    data[from..].windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

/// Inflate one raw DEFLATE stream; with `to_end`, the stream may stop before the slice does
/// (Python's `decompressobj`), else it must end inside it (Python's `zlib.decompress`).
/// Returns the bytes and how many input bytes the stream used.
fn inflate(input: &[u8], limit: usize) -> Option<(Vec<u8>, usize)> {
    let mut d = Decompress::new(false);
    let mut out: Vec<u8> = Vec::with_capacity(input.len().saturating_mul(4).clamp(1024, limit.max(1024)));
    loop {
        if out.len() == out.capacity() {
            if out.len() >= limit {
                return None; // more than Python's max_length
            }
            out.reserve(out.len().max(64 * 1024).min(limit - out.len()));
        }
        let (in_before, out_before) = (d.total_in(), d.total_out());
        let consumed = in_before as usize;
        let status = d.decompress_vec(&input[consumed..], &mut out, FlushDecompress::None).ok()?;
        if status == Status::StreamEnd {
            return Some((out, d.total_in() as usize));
        }
        let stalled = d.total_in() == in_before && d.total_out() == out_before;
        if stalled && out.len() < out.capacity() {
            return None; // all input used without the end of the stream: truncated
        }
    }
}

/// Every block of a SolidWorks container, in file order.
pub fn read_blocks(data: &[u8]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut pos = 0;
    let end = data.len();
    while let Some(i) = find(data, MARKER, pos) {
        let head = i + MARKER.len();
        if head + 20 > end {
            break;
        }
        let comp = u32_at(data, head + 8) as usize;
        let raw = u32_at(data, head + 12) as usize;
        let name_len = u32_at(data, head + 16);
        let name_start = head + 20;
        let payload_start = name_start + name_len as usize;
        if !(0 < name_len && name_len <= MAX_NAME) || raw > MAX_RAW || payload_start + comp > end {
            pos = i + 1;
            continue;
        }
        let name: Vec<u8> = data[name_start..payload_start].iter().map(|b| b.rotate_right(4)).collect();
        if !name.iter().all(|b| (32..127).contains(b)) {
            pos = i + 1;
            continue;
        }
        let payload = &data[payload_start..payload_start + comp];
        let (content, used) = if comp == raw {
            (Some(payload.to_vec()), comp)
        } else if let Some((out, _)) = inflate(payload, MAX_RAW) {
            (Some(out), comp)
        } else {
            // Some headers state sizes that do not match their payload (the raw size is then
            // exactly twice the compressed one). Decode the DEFLATE stream to its own end.
            let window = &data[payload_start..(payload_start + MAX_RAW).min(end)];
            match inflate(window, MAX_RAW) {
                Some((out, used)) => (Some(out), used),
                None => (None, 0),
            }
        };
        let raw_size = match &content {
            Some(c) if !c.is_empty() => c.len(),
            _ => raw,
        };
        blocks.push(Block {
            name: String::from_utf8_lossy(&name).into_owned(),
            offset: i,
            comp_size: used,
            raw_size,
            data: content,
        });
        pos = payload_start + used;
    }
    blocks
}

pub fn read_file(path: &Path) -> Result<Vec<Block>> {
    Ok(read_blocks(&std::fs::read(path)?))
}

/// The last decodable block with this name (earlier copies are stale).
pub fn block<'a>(blocks: &'a [Block], name: &str) -> Option<&'a [u8]> {
    blocks.iter().rev().find(|b| b.name == name && b.data.is_some()).and_then(|b| b.data.as_deref())
}

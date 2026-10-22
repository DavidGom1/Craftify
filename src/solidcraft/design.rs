//! Changes made straight to a saved design, where a SolidCraft command would compute the whole
//! design again: a `.solidcraft` file is a zip whose `document.json` holds the timeline, next to
//! the STEP files it imports (`blobs/`) and a cache of the computed model (`cache/`).

use std::io::{Read, Write};
use std::path::Path;

use anyhow::Result;
use serde_json::Value;

/// Change a design's `document.json`. Its cache goes: it belongs to the document as it was, and
/// SolidCraft makes it again when it opens the design.
pub fn edit_document(design: &Path, edit: impl FnOnce(&mut Value) -> Result<()>) -> Result<()> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(design)?)?;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            let name = f.name()?.into_owned();
            if f.is_dir() || name.starts_with("cache/") {
                continue;
            }
            let mut data = Vec::new();
            f.read_to_end(&mut data)?;
            entries.push((name, data));
        }
    }
    let doc =
        entries.iter_mut().find(|(n, _)| n == "document.json").ok_or_else(|| anyhow::anyhow!("{}: no document.json", design.display()))?;
    let mut value: Value = serde_json::from_slice(&doc.1)?;
    edit(&mut value)?;
    doc.1 = serde_json::to_vec(&value)?;
    let tmp = design.with_extension("solidcraft.tmp");
    {
        let mut out = zip::ZipWriter::new(std::fs::File::create(&tmp)?);
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in &entries {
            out.start_file(name.as_str(), options)?;
            out.write_all(data)?;
        }
        out.finish()?;
    }
    std::fs::rename(&tmp, design)?;
    Ok(())
}

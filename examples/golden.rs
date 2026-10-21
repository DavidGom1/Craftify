//! The Rust STEP repairs on the reference parts, for comparing with the Python ones.
//!
//! usage: cargo run --release --example golden -- <raw STEP folder> <ir cache folder> <out folder>
//! (the raw STEP files and the Python outputs come from a Python run of the same repairs).

use std::path::PathBuf;
use std::time::Instant;

use craftify::solidcraft::transform::Y_UP_TO_Z_UP;
use craftify::step::{fix, mesh_fix};
use craftify::sw::ir;
use craftify::util::{py_repr, read_latin1};

fn write(p: &std::path::Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text.chars().map(|c| c as u8).collect::<Vec<u8>>()).unwrap();
}

fn main() -> anyhow::Result<()> {
    let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let (raw_dir, cache, out) = (&args[0], &args[1], &args[2]);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(raw_dir)?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for raw in entries {
        let stem = raw.file_stem().unwrap().to_string_lossy().into_owned();
        let text = read_latin1(&raw)?;
        let model: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(cache.join(format!("{stem}.json")))?)?;
        let model = model.get("model").cloned().unwrap_or(model);
        let mesh = ir::mesh(&model)?;
        let t0 = Instant::now();
        let d = out.join("rs").join(&stem);
        let (fixed, _) = fix::repair_text(&text)?;
        write(&d.join("rules.step"), &fixed);
        for (name, opts) in mesh_fix::REPAIRS {
            let t = match mesh_fix::repair_text(&text, &mesh, opts) {
                Ok((t, _)) => t,
                Err(e) => format!("ERROR {e}"),
            };
            write(&d.join(format!("{name}.step")), &t);
        }
        write(&d.join("rotated.step"), &fix::rotate_text(&text, &Y_UP_TO_Z_UP)?);
        println!("{stem}: {} triangles, vol {}, {:.1} s", mesh.triangles.len(), py_repr(mesh.volume()), t0.elapsed().as_secs_f64());
    }
    Ok(())
}

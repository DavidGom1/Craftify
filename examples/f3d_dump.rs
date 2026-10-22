//! What the offline reader finds in a Fusion file: each document's components (name, bodies,
//! placements) and which components stand for other documents. For checking the reader.
//!
//! usage: cargo run --release --example f3d_dump -- <design.f3d | project.f3z>

use std::io::Read;

use craftify::fusion::f3d::{open_f3d, read_assembly};

fn dump(name: &str, data: &[u8]) -> anyhow::Result<()> {
    let f = open_f3d(data)?;
    let asm = read_assembly(&f)?;
    println!("== {name}: root {:?}, {} components, {} placements", asm.root, asm.components.len(), asm.placements.len());
    for (id, c) in &asm.components {
        let placed = asm.placements.iter().filter(|p| p.target == Some(*id)).count();
        let ext: Vec<&str> = asm.placements.iter().filter(|p| p.target == Some(*id)).filter_map(|p| p.external.as_deref()).collect();
        println!(
            "  {id:>7} {:40} bodies {:3} placed {placed} {}",
            c.name,
            c.bodies.len(),
            if ext.is_empty() { String::new() } else { format!("external {ext:?}") }
        );
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("a .f3d or .f3z file");
    if path.to_lowercase().ends_with(".f3z") {
        let mut z = zip::ZipArchive::new(std::fs::File::open(&path)?)?;
        for i in 0..z.len() {
            let mut f = z.by_index(i)?;
            let name = f.name()?.into_owned();
            if name.to_lowercase().ends_with(".f3d") && !name.contains('/') {
                let mut data = Vec::new();
                f.read_to_end(&mut data)?;
                dump(&name, &data)?;
            }
        }
    } else {
        dump(&path, &std::fs::read(&path)?)?;
    }
    Ok(())
}

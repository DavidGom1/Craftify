//! Rebuild one component's timeline as a SolidCraft script, for checking the rebuild by hand.
//!
//! usage: cargo run --release --example rebuild_try -- <design.f3d> <component name> <its part .step> <out script.json>

use std::path::Path;

use craftify::fusion::f3d::{open_f3d, read_assembly};
use craftify::fusion::rebuild::{fillet_edges, rebuild};
use craftify::fusion::timeline::Feature;
use craftify::step::Step;
use craftify::util::read_latin1;
use serde_json::json;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let f = open_f3d(&std::fs::read(&a[0])?)?;
    let asm = read_assembly(&f)?;
    let (cid, _) = asm.components.iter().find(|(_, c)| c.name == a[1]).ok_or_else(|| anyhow::anyhow!("no component {}", a[1]))?;
    let timeline = asm.timelines.get(cid).cloned().unwrap_or_default();
    let sketches = asm.sketches.get(cid).map(Vec::as_slice).unwrap_or(&[]);
    let step = Step::new(&read_latin1(Path::new(&a[2]))?);
    for f in &timeline {
        if let Feature::Fillet { radius } = f {
            println!("fillet {} edges at {:?}", radius.expression, fillet_edges(&step, radius.value));
        }
    }
    match rebuild(&timeline, sketches, &step) {
        Ok(r) => {
            let mut cmds = vec![json!({"command": "file.new", "params": {"name": a[1]}})];
            cmds.extend(r.script.commands);
            std::fs::write(&a[3], serde_json::to_string_pretty(&json!({ "commands": cmds }))?)?;
            println!("{} features, script {}", r.features, a[3]);
        }
        Err(e) => println!("not rebuilt: {e}"),
    }
    Ok(())
}

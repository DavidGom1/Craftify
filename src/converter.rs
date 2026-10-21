//! One entry point for every source CAD, chosen by the file's extension.

use std::path::Path;

use anyhow::Result;

use crate::report::{PartResult, Report};
use crate::util::suffix_lower;
use crate::{fusion, sw};

pub const SOLIDWORKS: [&str; 2] = [".sldasm", ".sldprt"];
pub const FUSION: [&str; 2] = [".f3d", ".f3z"];

pub fn supported(src: &Path) -> bool {
    let ext = suffix_lower(src);
    SOLIDWORKS.contains(&ext.as_str()) || FUSION.contains(&ext.as_str())
}

pub fn is_fusion(src: &Path) -> bool {
    FUSION.contains(&suffix_lower(src).as_str())
}

/// What a conversion is doing: `done` of `total` parts at each stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Fusion is exporting the design.
    Export,
    Start,
    Part,
    Assembly,
    Picture,
}

pub type Progress = dyn Fn(Stage, usize, usize, Option<&PartResult>);

/// Where a conversion reports to: its log lines and its progress.
pub struct Hooks {
    pub log: Box<dyn Fn(&str)>,
    pub progress: Option<Box<Progress>>,
}

impl Hooks {
    /// Log lines to the terminal, no progress.
    pub fn print() -> Hooks {
        Hooks { log: Box::new(|l| println!("{l}")), progress: None }
    }

    pub fn log(&self, line: &str) {
        (self.log)(line);
    }

    pub fn tell(&self, stage: Stage, done: usize, total: usize, part: Option<&PartResult>) {
        if let Some(p) = &self.progress {
            p(stage, done, total, part);
        }
    }
}

pub struct Options<'a> {
    /// Turn SolidWorks' Y-up models upright (Fusion designs keep their own axes).
    pub up_y: bool,
    pub solidcraft_cli: Option<&'a str>,
    pub cadmpeg: Option<&'a str>,
    /// Draw a picture of the result.
    pub picture: bool,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Options { up_y: true, solidcraft_cli: None, cadmpeg: None, picture: true }
    }
}

pub fn kind(src: &Path) -> &'static str {
    match suffix_lower(src).as_str() {
        ".sldasm" => "Ensamblaje de SolidWorks",
        ".sldprt" => "Pieza de SolidWorks",
        ".f3z" => "Proyecto de Fusion (con referencias)",
        ".f3d" => "Diseño de Fusion",
        _ => "Fichero",
    }
}

pub fn convert(src: &Path, out_root: &Path, opts: &Options, hooks: &Hooks) -> Result<Report> {
    let src = crate::util::native(src);
    let out_root = crate::util::native(out_root);
    let cli = crate::solidcraft::script::find_cli(opts.solidcraft_cli)?;
    let tools = sw::convert::Tools { cli, cadmpeg: sw::convert::find_cadmpeg(opts.cadmpeg)? };
    if is_fusion(&src) {
        // Fusion designs keep their own axes: `up_y` is for SolidWorks.
        return fusion::offline::convert_fusion_offline(&src, &out_root, &tools.cli, &tools.cadmpeg, opts.picture, hooks);
    }
    if suffix_lower(&src) == ".sldprt" {
        sw::convert::convert_single_part(&src, &out_root, opts.up_y, &tools, opts.picture, hooks)
    } else {
        sw::convert::convert_assembly(&src, &out_root, opts.up_y, &tools, opts.picture, hooks)
    }
}

/// The preview picture the source CAD saved inside the file.
pub fn preview_png(src: &Path) -> Option<Vec<u8>> {
    if is_fusion(src) { fusion::preview_png(src) } else { sw::convert::preview_png(src) }
}

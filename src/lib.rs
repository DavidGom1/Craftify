//! Craftify: SolidWorks assemblies and parts, and Fusion designs, into SolidCraft designs.
//!
//! A port of the Python converter (`cad2solidcraft`): the same readers, the same STEP repairs
//! and the same checks against the source CAD, so both give the same designs.

pub mod converter;
pub mod fusion;
pub mod gui;
pub mod mesh;
pub mod pyset;
pub mod report;
pub mod settings;
pub mod solidcraft;
pub mod step;
pub mod sw;
pub mod util;

pub use converter::{Progress, Stage, convert, kind, preview_png};
pub use report::{PartResult, Report};

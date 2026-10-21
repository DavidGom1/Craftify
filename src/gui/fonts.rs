//! SolidCraft's interface faces: IBM Plex Sans at real weights and Plex Mono.
//!
//! The set-up follows SolidCraft 0.18 (`crates/ui-egui/src/fonts.rs`, MIT OR Apache-2.0); the
//! fonts are IBM Plex (SIL Open Font License 1.1, assets/fonts/OFL-IBMPlex.txt).

use std::sync::Arc;

use egui::{FontDefinitions, FontFamily, FontId};

const PLEX_SANS: &[u8] = include_bytes!("../../assets/fonts/IBMPlexSans-Variable.ttf");
const PLEX_MONO: &[u8] = include_bytes!("../../assets/fonts/IBMPlexMono-Regular.ttf");
const WEIGHTS: [(&str, f32); 3] = [("plex-sans", 400.0), ("ui-semibold", 600.0), ("ui-bold", 700.0)];

pub fn semibold_family() -> FontFamily {
    FontFamily::Name("ui-semibold".into())
}

pub fn bold_family() -> FontFamily {
    FontFamily::Name("ui-bold".into())
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, semibold_family())
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, bold_family())
}

/// Plex Sans at 400/600/700 and Plex Mono, followed by egui's symbol and emoji fallbacks.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let families = [FontFamily::Proportional, semibold_family(), bold_family()];
    for ((name, weight), family) in WEIGHTS.into_iter().zip(families) {
        let mut data = egui::FontData::from_static(PLEX_SANS);
        data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
        fonts.font_data.insert(name.to_owned(), Arc::new(data));
        fonts.families.insert(family, std::iter::once(name.to_owned()).chain(fallbacks.iter().cloned()).collect());
    }
    fonts.font_data.insert("plex-mono".to_owned(), Arc::new(egui::FontData::from_static(PLEX_MONO)));
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "plex-mono".to_owned());
    ctx.set_fonts(fonts);
}

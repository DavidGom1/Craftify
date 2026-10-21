//! SolidCraft's look: the colours, sizes and widget style of its interface, in light and dark.
//!
//! The values are copied from SolidCraft 0.18 (`crates/ui-egui/src/theme.rs` and
//! `assets/styles/ui/palettes.rs`, https://github.com/tangency-labs/solidcraft, MIT OR
//! Apache-2.0, © SolidCraft contributors), keeping only the tokens this window uses, so Craftify
//! sits next to SolidCraft as one of its own dialogs would. See NOTICE.md.

use egui::{Color32, CornerRadius, Stroke};

/// The subset of SolidCraft's `Tokens` this window draws with.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub dark: bool,
    pub space_4: f32,
    pub space_6: f32,
    pub space_8: f32,
    pub space_12: f32,
    pub space_16: f32,
    pub radius_sm: u8,
    pub radius_md: u8,
    pub radius_lg: u8,
    pub font_small: f32,
    pub font_caption: f32,
    pub font_body: f32,
    pub font_label: f32,
    pub font_button: f32,
    pub font_heading: f32,
    pub font_subtitle: f32,
    pub font_title: f32,
    pub control_height: f32,
    pub control_radius: u8,
    pub form_header_height: f32,
    pub form_button_height: f32,
    pub form_success: Color32,
    pub form_success_fill: Color32,
    pub form_error: Color32,
    pub form_error_fill: Color32,
    pub app_bar: Color32,
    pub app_bar_text: Color32,
    pub app_bar_separator: Color32,
    pub app_bar_hover: Color32,
    pub toolbar: Color32,
    pub panel: Color32,
    pub panel_header: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub error: Color32,
    pub warning: Color32,
    pub hover: Color32,
    pub field: Color32,
    pub dialog_body: Color32,
    pub dialog_footer: Color32,
    pub dialog_rule: Color32,
    pub dialog_border: Color32,
    pub dialog_label: Color32,
    pub heading_bg: Color32,
    pub control_fill: Color32,
    pub control_border: Color32,
    pub selected_ink: Color32,
    pub selected_bg: Color32,
    /// The SolidCraft mark: light, mid and dark faces, and its edge.
    pub logo: [Color32; 4],
}

impl Tokens {
    pub const fn light() -> Tokens {
        Tokens {
            dark: false,
            space_4: 4.0,
            space_6: 6.0,
            space_8: 8.0,
            space_12: 12.0,
            space_16: 16.0,
            radius_sm: 4,
            radius_md: 6,
            radius_lg: 8,
            font_small: 11.0,
            font_caption: 11.5,
            font_body: 12.0,
            font_label: 12.5,
            font_button: 13.0,
            font_heading: 14.0,
            font_subtitle: 15.0,
            font_title: 22.0,
            control_height: 28.0,
            control_radius: 6,
            form_header_height: 40.0,
            form_button_height: 28.0,
            form_success: Color32::from_rgb(39, 107, 44),
            form_success_fill: Color32::from_rgb(229, 242, 230),
            form_error: Color32::from_rgb(161, 52, 44),
            form_error_fill: Color32::from_rgb(252, 239, 237),
            app_bar: Color32::from_rgb(52, 56, 64),
            app_bar_text: Color32::from_rgb(230, 234, 242),
            app_bar_separator: Color32::from_rgb(136, 145, 160),
            app_bar_hover: Color32::from_rgba_unmultiplied_const(255, 255, 255, 28),
            toolbar: Color32::from_rgb(246, 247, 249),
            panel: Color32::from_rgb(250, 251, 252),
            panel_header: Color32::from_rgb(236, 238, 242),
            border: Color32::from_rgb(208, 212, 220),
            text: Color32::from_rgb(38, 42, 50),
            text_dim: Color32::from_rgb(110, 116, 128),
            accent: Color32::from_rgb(38, 120, 218),
            accent_soft: Color32::from_rgb(214, 230, 250),
            on_accent: Color32::WHITE,
            error: Color32::from_rgb(214, 54, 54),
            warning: Color32::from_rgb(222, 150, 30),
            hover: Color32::from_rgb(226, 234, 246),
            field: Color32::WHITE,
            dialog_body: Color32::from_rgb(255, 255, 255),
            dialog_footer: Color32::from_rgb(244, 246, 249),
            dialog_rule: Color32::from_rgb(230, 234, 240),
            dialog_border: Color32::from_rgb(213, 219, 227),
            dialog_label: Color32::from_rgb(58, 70, 86),
            heading_bg: Color32::from_rgb(218, 223, 231),
            control_fill: Color32::from_rgb(255, 255, 255),
            control_border: Color32::from_rgb(195, 203, 214),
            selected_ink: Color32::from_rgb(26, 80, 127),
            selected_bg: Color32::from_rgb(232, 241, 250),
            logo: [
                Color32::from_rgb(96, 158, 236),
                Color32::from_rgb(58, 136, 238),
                Color32::from_rgb(55, 108, 197),
                Color32::WHITE,
            ],
        }
    }

    pub const fn dark() -> Tokens {
        let mut t = Tokens::light();
        t.dark = true;
        t.app_bar = Color32::from_rgb(28, 31, 38);
        t.toolbar = Color32::from_rgb(39, 43, 52);
        t.panel = Color32::from_rgb(39, 43, 52);
        t.panel_header = Color32::from_rgb(47, 52, 62);
        t.border = Color32::from_rgb(63, 69, 82);
        t.text = Color32::from_rgb(222, 226, 232);
        t.text_dim = Color32::from_rgb(148, 155, 168);
        t.accent = Color32::from_rgb(58, 136, 238);
        t.accent_soft = Color32::from_rgb(42, 70, 110);
        t.dialog_body = Color32::from_rgb(59, 68, 83);
        t.dialog_footer = Color32::from_rgb(47, 52, 62);
        t.dialog_rule = Color32::from_rgb(53, 59, 71);
        t.dialog_border = Color32::from_rgb(72, 83, 99);
        t.dialog_label = Color32::from_rgb(180, 187, 200);
        t.heading_bg = Color32::from_rgb(38, 44, 55);
        t.control_fill = Color32::from_rgb(39, 43, 52);
        t.control_border = Color32::from_rgb(74, 82, 98);
        t.selected_ink = Color32::from_rgb(156, 199, 245);
        t.selected_bg = Color32::from_rgb(42, 70, 110);
        t.error = Color32::from_rgb(240, 100, 96);
        t.warning = Color32::from_rgb(236, 170, 70);
        t.hover = Color32::from_rgb(54, 61, 74);
        t.field = Color32::from_rgb(30, 34, 41);
        t.form_error = t.error;
        t.form_error_fill = Color32::from_rgb(110, 50, 54);
        t.form_success = Color32::from_rgb(143, 219, 154);
        t.form_success_fill = Color32::from_rgb(44, 74, 51);
        t
    }

    pub fn themed(dark: bool) -> Tokens {
        if dark { Tokens::dark() } else { Tokens::light() }
    }

    pub fn radius(&self) -> CornerRadius {
        CornerRadius::same(self.radius_md)
    }
}

/// SolidCraft's `theme::visuals`, for the egui widgets this window uses as they are.
fn visuals(t: &Tokens) -> egui::Visuals {
    let mut v = if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = t.panel;
    v.window_fill = t.panel;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.panel_header;
    v.override_text_color = Some(t.text);
    v.widgets.inactive.weak_bg_fill = t.panel_header;
    v.widgets.inactive.bg_fill = t.panel_header;
    v.widgets.active.weak_bg_fill = t.accent_soft;
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.open.weak_bg_fill = t.panel_header;
    v.window_corner_radius = CornerRadius::same(t.radius_md);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(t.control_radius);
    }
    v
}

/// SolidCraft's `theme::apply`, with its text sizes.
pub fn apply(ctx: &egui::Context, t: &Tokens) {
    ctx.set_theme(if t.dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.set_visuals(visuals(t));
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(t.space_6, t.space_4);
        s.spacing.button_padding = egui::vec2(t.space_6, 3.0);
        s.spacing.interact_size.y = t.control_height;
        use egui::{FontFamily, FontId, TextStyle};
        s.text_styles = [
            (TextStyle::Small, FontId::new(t.font_small, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(t.font_label, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(t.font_button, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(t.font_subtitle, super::fonts::semibold_family())),
            (TextStyle::Monospace, FontId::new(t.font_body, FontFamily::Monospace)),
        ]
        .into();
    });
}

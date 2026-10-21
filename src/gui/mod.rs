//! The converter window, drawn with egui in SolidCraft's own style.
//!
//! The same window as the Python/Flet one: the file, the output folder and the options on the
//! left; the two pictures, the parts as they finish and the technical details on the right.
//! The conversion runs on a worker thread and reports back through a channel.

pub mod fonts;
pub mod theme;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use egui::{Align, Align2, Color32, CornerRadius, Layout, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use serde_json::{Map, Value, json};

use crate::converter::{self, Hooks, Options, Stage};
use crate::report::{PartResult, Report};
use crate::settings;
use crate::sw::assembly::read_assembly;
use crate::sw::convert::PREVIEW_RESULT;
use crate::util::{self, suffix_lower};
use theme::Tokens;

pub const TITLE: &str = "Craftify";
pub const ICON_PNG: &[u8] = include_bytes!("../../assets/icon.png");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Waiting,
    Running,
    Ok,
    Warn,
    Error,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            State::Waiting => "En espera",
            State::Running => "Convirtiendo",
            State::Ok => "Coincide",
            State::Warn => "No coincide",
            State::Error => "Error",
        }
    }
    fn glyph(self) -> &'static str {
        match self {
            State::Waiting => "•",
            State::Running => "▶",
            State::Ok => "✔",
            State::Warn => "⚠",
            State::Error => "✖",
        }
    }
    /// Ink and fill of the state's chip.
    fn colors(self, t: &Tokens) -> (Color32, Color32) {
        match self {
            State::Waiting => (t.text_dim, t.panel_header),
            State::Running => (t.selected_ink, t.accent_soft),
            State::Ok => (t.form_success, t.form_success_fill),
            State::Warn => (if t.dark { t.warning } else { Color32::from_rgb(138, 88, 0) }, t.warning.gamma_multiply(0.22)),
            State::Error => (t.form_error, t.form_error_fill),
        }
    }
}

struct Row {
    stem: String,
    count: usize,
    state: State,
    part: Option<PartResult>,
}

enum Msg {
    Log(String),
    Progress(Stage, usize, usize, Option<PartResult>),
    Done(Report),
    Fail(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Good,
    Warn,
    Bad,
}

struct Toast {
    text: String,
    tone: Tone,
    since: Instant,
}

/// Parts table: (title, flex, numeric).
const COLUMNS: [(&str, f32, bool); 6] = [
    ("Pieza", 4.0, false),
    ("Estado", 3.0, false),
    ("Volumen", 2.0, true),
    ("Original", 2.0, true),
    ("Corrección", 3.0, false),
    ("Tiempo", 1.3, true),
];

pub struct App {
    settings: Map<String, Value>,
    theme_pref: String,
    tokens: Tokens,
    applied_dark: Option<bool>,
    source: String,
    out: String,
    up_y: bool,
    src_name: String,
    src_folder: String,
    src_info: String,
    rows: Vec<Row>,
    selected: Option<String>,
    report: Option<Report>,
    running: bool,
    /// None: indeterminate.
    progress: Option<f32>,
    status: String,
    log: String,
    pic_src: Option<egui::TextureHandle>,
    pic_out: Option<egui::TextureHandle>,
    pic_src_note: &'static str,
    logo: egui::TextureHandle,
    toast: Option<Toast>,
    rx: Option<Receiver<Msg>>,
    shot: Option<Shot>,
}

/// Automated checks of the window: CAD2SC_SOURCE / CAD2SC_OUT / CAD2SC_AUTOSTART /
/// CAD2SC_DONE_FILE as in the Python window, plus CRAFTIFY_SCREENSHOT (a PNG of the window,
/// taken when it settles) and CRAFTIFY_CLOSE (close after it).
struct Shot {
    path: PathBuf,
    close: bool,
    frames: u32,
    asked: bool,
}

fn texture(ctx: &egui::Context, name: &str, png: &[u8]) -> Option<egui::TextureHandle> {
    let img = image::load_from_memory(png).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR))
}

fn setting_str(s: &Map<String, Value>, key: &str) -> Option<String> {
    s.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned)
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let ctx = &cc.egui_ctx;
        fonts::install(ctx);
        let settings = settings::load();
        let theme_pref =
            std::env::var("CRAFTIFY_THEME").ok().or_else(|| setting_str(&settings, "theme")).unwrap_or_else(|| "system".into());
        let source = std::env::var("CAD2SC_SOURCE").ok().or_else(|| setting_str(&settings, "source")).unwrap_or_default();
        let out =
            std::env::var("CAD2SC_OUT").ok().or_else(|| setting_str(&settings, "out")).unwrap_or_else(|| util::s(&settings::default_out()));
        let up_y = settings.get("up_y").and_then(Value::as_bool).unwrap_or(true);
        let logo =
            texture(ctx, "logo", ICON_PNG).unwrap_or_else(|| ctx.load_texture("logo", egui::ColorImage::example(), Default::default()));
        let shot = std::env::var_os("CRAFTIFY_SCREENSHOT").map(|p| Shot {
            path: PathBuf::from(p),
            close: std::env::var_os("CRAFTIFY_CLOSE").is_some(),
            frames: 0,
            asked: false,
        });
        let mut app = App {
            settings,
            theme_pref,
            tokens: Tokens::light(),
            applied_dark: None,
            source,
            out,
            up_y,
            src_name: "Nada elegido todavía".into(),
            src_folder: String::new(),
            src_info: "SolidWorks: ensamblaje .SLDASM o pieza .SLDPRT. Fusion: diseño .f3d o proyecto .f3z (no hace falta tener Fusion)"
                .into(),
            rows: Vec::new(),
            selected: None,
            report: None,
            running: false,
            progress: Some(0.0),
            status: "Elige un fichero para empezar.".into(),
            log: String::new(),
            pic_src: None,
            pic_out: None,
            pic_src_note: "Aparecerá al convertir",
            logo,
            toast: None,
            rx: None,
            shot,
        };
        app.show_source(ctx);
        if std::env::var_os("CAD2SC_AUTOSTART").is_some() {
            app.start(ctx);
        }
        app
    }

    fn dark(&self, ctx: &egui::Context) -> bool {
        match self.theme_pref.as_str() {
            "dark" => true,
            "light" => false,
            _ => ctx.system_theme() == Some(egui::Theme::Dark),
        }
    }

    fn remember(&mut self) {
        self.settings.insert("source".into(), json!(self.source));
        self.settings.insert("out".into(), json!(self.out));
        self.settings.insert("up_y".into(), json!(self.up_y));
        self.settings.insert("theme".into(), json!(self.theme_pref));
        settings::save(&self.settings);
    }

    fn toast(&mut self, text: impl Into<String>, tone: Tone) {
        self.toast = Some(Toast { text: text.into(), tone, since: Instant::now() });
    }

    // ------------------------------------------------------------------ choices

    fn pick_source(&mut self, ctx: &egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Fichero de SolidWorks o de Fusion")
            .add_filter("SolidWorks y Fusion", &["sldasm", "SLDASM", "sldprt", "SLDPRT", "f3d", "F3D", "f3z", "F3Z"]);
        if let Some(dir) = Path::new(&self.source).parent().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(p) = dialog.pick_file() {
            self.source = util::s(&p);
            self.remember();
            self.show_source(ctx);
        }
    }

    fn pick_out(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Carpeta de salida");
        if Path::new(&self.out).is_dir() {
            dialog = dialog.set_directory(&self.out);
        }
        if let Some(p) = dialog.pick_folder() {
            self.out = util::s(&p);
            self.remember();
        }
    }

    fn show_source(&mut self, ctx: &egui::Context) {
        self.rows.clear();
        self.selected = None;
        let src = PathBuf::from(&self.source);
        if self.source.is_empty() || !src.is_file() {
            return;
        }
        self.src_name = util::stem(&src);
        self.pic_src = converter::preview_png(&src).and_then(|png| texture(ctx, "source", &png));
        self.pic_src_note = if self.pic_src.is_some() { "" } else { "Este fichero no guarda vista previa" };
        self.pic_out = None;
        let folder = src.parent().map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        self.src_folder = format!("Carpeta: {}", folder.unwrap_or_default());
        match suffix_lower(&src).as_str() {
            ".sldprt" => {
                self.src_info = "Pieza suelta".into();
                self.rows.push(Row { stem: self.src_name.clone(), count: 1, state: State::Waiting, part: None });
            }
            ".f3d" | ".f3z" => match crate::fusion::offline::part_names(&src) {
                Ok(stems) => {
                    self.src_info = format!("{} · {} pieza(s)", converter::kind(&src), stems.len());
                    for stem in stems {
                        self.rows.push(Row { stem, count: 1, state: State::Waiting, part: None });
                    }
                }
                Err(e) => self.src_info = format!("No se puede leer: {e}"),
            },
            _ => match read_assembly(&src) {
                Ok(asm) => {
                    for i in &asm.instances {
                        match self.rows.iter_mut().find(|r| r.stem == i.stem) {
                            Some(r) => r.count += 1,
                            None => self.rows.push(Row { stem: i.stem.clone(), count: 1, state: State::Waiting, part: None }),
                        }
                    }
                    self.src_info = format!("Ensamblaje · {} piezas · {} instancias", self.rows.len(), asm.instances.len());
                }
                Err(e) => {
                    self.src_info = format!("No se puede leer: {e}");
                    return;
                }
            },
        }
        self.status = "Listo para convertir.".into();
    }

    // ------------------------------------------------------------------ conversion

    fn start(&mut self, ctx: &egui::Context) {
        let src = PathBuf::from(&self.source);
        if self.source.is_empty() || !converter::supported(&src) || !src.is_file() {
            self.toast("Elige primero un fichero de SolidWorks (.SLDASM, .SLDPRT) o de Fusion (.f3d, .f3z).", Tone::Bad);
            return;
        }
        self.report = None;
        self.running = true;
        self.log.clear();
        self.show_source(ctx);
        self.progress = None; // indeterminate until the first step reports
        self.status = format!("Convirtiendo {}…", util::stem(&src));
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let (out, up_y, ctx) = (PathBuf::from(&self.out), self.up_y, ctx.clone());
        std::thread::spawn(move || work(src, out, up_y, tx, ctx));
    }

    fn receive(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else { return };
        let msgs: Vec<Msg> = rx.try_iter().collect();
        for m in msgs {
            match m {
                Msg::Log(l) => {
                    self.log.push_str(&l);
                    self.log.push('\n');
                }
                Msg::Progress(stage, done, total, part) => self.on_progress(stage, done, total, part),
                Msg::Done(r) => self.finish(ctx, r),
                Msg::Fail(e) => self.fail(&e),
            }
        }
    }

    fn set_state(&mut self, stem: &str, state: State) {
        if let Some(r) = self.rows.iter_mut().find(|r| r.stem == stem) {
            r.state = state;
        }
    }

    fn on_progress(&mut self, stage: Stage, mut done: usize, total: usize, part: Option<PartResult>) {
        let steps = total + 2;
        match stage {
            Stage::Export => {
                self.status = "Exportando desde Fusion… (Fusion se abre si no lo está)".into();
                self.progress = None;
                return;
            }
            Stage::Start => {
                if let Some(first) = self.rows.first().map(|r| r.stem.clone()) {
                    self.set_state(&first, State::Running);
                }
            }
            Stage::Part => {
                if let Some(p) = part {
                    let state = if !p.ok {
                        State::Error
                    } else if p.matches_source == Some(false) {
                        State::Warn
                    } else {
                        State::Ok
                    };
                    self.status = format!("Pieza {done} de {total}: {}", p.stem);
                    match self.rows.iter_mut().find(|r| r.stem == p.stem) {
                        Some(r) => {
                            r.state = state;
                            r.part = Some(p);
                        }
                        None => self.rows.push(Row { stem: p.stem.clone(), count: 1, state, part: Some(p) }),
                    }
                    if let Some(next) = self.rows.iter_mut().find(|r| r.state == State::Waiting) {
                        next.state = State::Running;
                    }
                }
            }
            Stage::Assembly => {
                self.status = "Montando el ensamblaje…".into();
                done = total + 1;
            }
            Stage::Picture => {
                self.status = "Dibujando el resultado…".into();
                done = total + 2;
            }
        }
        self.progress = Some(done as f32 / steps.max(1) as f32);
    }

    fn finish(&mut self, ctx: &egui::Context, report: Report) {
        self.running = false;
        self.progress = Some(1.0);
        let placed = if report.instances > 1 {
            format!(" · {} de {} instancias colocadas", report.placed, report.instances)
        } else {
            String::new()
        };
        self.status = format!("Terminado en {:.0} s{placed}.", report.seconds);
        if let Some(output) = &report.output {
            let f = Path::new(output).parent().map(|d| d.join(PREVIEW_RESULT));
            if let Some(png) = f.and_then(|f| std::fs::read(f).ok()) {
                self.pic_out = texture(ctx, "result", &png);
            }
        }
        for w in &report.warnings {
            self.log.push_str(&format!("Aviso: {w}\n"));
        }
        if report.ok() {
            self.toast("Conversión terminada: todo coincide con el original.", Tone::Good);
        } else {
            self.toast("Conversión terminada con avisos: revisa las piezas marcadas.", Tone::Warn);
        }
        if let Some(f) = std::env::var_os("CAD2SC_DONE_FILE") {
            let _ = std::fs::write(f, &self.status);
        }
        self.report = Some(report);
    }

    fn fail(&mut self, message: &str) {
        self.running = false;
        self.progress = Some(0.0);
        self.status = "La conversión no ha terminado.".into();
        self.log.push_str(&format!("Error: {message}\n"));
        self.toast(message, Tone::Bad);
        if let Some(f) = std::env::var_os("CAD2SC_DONE_FILE") {
            let _ = std::fs::write(f, format!("ERROR: {message}"));
        }
    }

    fn open_result(&self) {
        let Some(output) = self.report.as_ref().and_then(|r| r.output.clone()) else { return };
        match settings::find_solidcraft() {
            Some(exe) => {
                let _ = std::process::Command::new(exe).arg(&output).spawn();
            }
            None => shell_open(Path::new(&output)),
        }
    }

    fn open_folder(&self) {
        if let Some(dir) = self.report.as_ref().and_then(|r| r.output.as_ref()).and_then(|o| Path::new(o).parent().map(Path::to_path_buf)) {
            shell_open(&dir);
        }
    }

    // ------------------------------------------------------------------ drawing

    fn app_bar(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        ui.horizontal_centered(|ui| {
            ui.add(egui::Image::new(&self.logo).fit_to_exact_size(Vec2::splat(26.0)));
            ui.add_space(t.space_4);
            ui.label(RichText::new(TITLE).font(fonts::semibold(t.font_subtitle)).color(t.app_bar_text));
            ui.add_space(t.space_8);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 18.0), Sense::hover());
            ui.painter().rect_filled(rect, 0, t.app_bar_separator.gamma_multiply(0.5));
            ui.add_space(t.space_8);
            ui.label(
                RichText::new("SolidWorks y Fusion a diseños de SolidCraft, comprobados con el original")
                    .size(t.font_body)
                    .color(t.app_bar_text.gamma_multiply(0.72)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let dark = t.dark;
                let label = if dark { "☀  Claro" } else { "🌙  Oscuro" };
                let galley_w = 92.0;
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley_w, 28.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, t.radius_md, t.app_bar_hover);
                }
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, egui::FontId::proportional(t.font_body), t.app_bar_text);
                if resp.on_hover_text("Cambiar entre claro y oscuro").clicked() {
                    self.theme_pref = if dark { "light".into() } else { "dark".into() };
                    self.remember();
                }
            });
        });
    }

    fn left(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = self.tokens;
        ui.spacing_mut().item_spacing.y = t.space_12;
        let src_name = self.src_name.clone();
        let (src_folder, src_info) = (self.src_folder.clone(), self.src_info.clone());
        let mut pick = false;
        card(ui, &t, Some(("1", "Fichero de SolidWorks o de Fusion")), |ui| {
            ui.add(egui::Label::new(RichText::new(src_name).font(fonts::semibold(t.font_subtitle))).truncate());
            if !src_folder.is_empty() {
                ui.add(egui::Label::new(RichText::new(src_folder).size(t.font_body).color(t.text_dim)).truncate())
                    .on_hover_text(Path::new(&self.source).parent().map(util::s).unwrap_or_default());
            }
            ui.label(RichText::new(src_info).size(t.font_body).color(t.text_dim));
            ui.add_space(t.space_4);
            pick = button(ui, &t, "📂  Elegir fichero", Kind::Secondary, true, None).clicked();
        });
        if pick {
            self.pick_source(ctx);
        }
        let mut pick_out = false;
        let out = self.out.clone();
        card(ui, &t, Some(("2", "Carpeta de salida")), |ui| {
            ui.add(egui::Label::new(RichText::new(&out).size(t.font_label)).wrap()).on_hover_text(&out);
            ui.add_space(t.space_4);
            pick_out = button(ui, &t, "Cambiar carpeta", Kind::Secondary, true, None).clicked();
        });
        if pick_out {
            self.pick_out();
        }
        let mut up = self.up_y;
        card(ui, &t, Some(("3", "Opciones")), |ui| {
            toggle(ui, &t, &mut up, "Poner de pie las piezas de SolidWorks");
            ui.label(RichText::new("SolidWorks usa Y como vertical y SolidCraft usa Z.").size(t.font_body).color(t.text_dim));
        });
        if up != self.up_y {
            self.up_y = up;
            self.remember();
        }
        let w = ui.available_width();
        if button(ui, &t, "▶  Convertir", Kind::Primary, !self.running, Some(Vec2::new(w, 40.0))).clicked() {
            self.start(ctx);
        }
        ui.spacing_mut().item_spacing.y = t.space_8;
        progress_bar(ui, &t, self.progress);
        ui.label(RichText::new(&self.status).size(t.font_label).color(t.text_dim));
        let ready = !self.running && self.report.as_ref().is_some_and(|r| r.output.is_some());
        if button(ui, &t, "↗  Abrir en SolidCraft", Kind::Secondary, ready, Some(Vec2::new(w, t.control_height + 4.0))).clicked() {
            self.open_result();
        }
        if button(ui, &t, "Abrir carpeta", Kind::Secondary, ready, Some(Vec2::new(w, t.control_height + 4.0))).clicked() {
            self.open_folder();
        }
    }

    fn right(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        ui.spacing_mut().item_spacing = Vec2::splat(t.space_12);
        let full = ui.available_width();
        let pic_w = (full - t.space_12) / 2.0;
        ui.horizontal(|ui| {
            for (caption, tex, note) in [
                ("Original · vista previa guardada", self.pic_src.clone(), self.pic_src_note),
                ("SolidCraft · resultado", self.pic_out.clone(), "Aparecerá al convertir"),
            ] {
                ui.allocate_ui(Vec2::new(pic_w, 290.0), |ui| {
                    ui.set_width(pic_w);
                    card(ui, &t, None, |ui| {
                        ui.label(RichText::new(caption).font(fonts::semibold(t.font_body)).color(t.text_dim));
                        let size = Vec2::new(ui.available_width(), 236.0);
                        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                        ui.painter().rect_filled(rect, t.radius_md, t.panel);
                        match tex {
                            Some(tex) => {
                                let img = tex.size_vec2();
                                let k = (rect.width() / img.x).min(rect.height() / img.y);
                                let r = Rect::from_center_size(rect.center(), img * k);
                                egui::Image::new(&tex).corner_radius(t.radius_md).paint_at(ui, r);
                            }
                            None => {
                                let text = if note.is_empty() { "Aparecerá al convertir" } else { note };
                                ui.painter().text(
                                    rect.center() - Vec2::new(0.0, 10.0),
                                    Align2::CENTER_CENTER,
                                    "🖼",
                                    egui::FontId::proportional(30.0),
                                    t.border,
                                );
                                ui.painter().text(
                                    rect.center() + Vec2::new(0.0, 18.0),
                                    Align2::CENTER_CENTER,
                                    text,
                                    egui::FontId::proportional(t.font_body),
                                    t.text_dim,
                                );
                            }
                        }
                    });
                });
            }
        });
        let details_h =
            46.0 + if ui.ctx().data(|d| d.get_temp::<bool>(egui::Id::new("details_open"))).unwrap_or(false) { 170.0 } else { 0.0 };
        let parts_h = (ui.available_height() - details_h - t.space_12).max(180.0);
        ui.allocate_ui(Vec2::new(full, parts_h), |ui| {
            ui.set_min_height(parts_h);
            self.parts_card(ui, &t);
        });
        self.details(ui, &t);
    }

    fn parts_card(&mut self, ui: &mut Ui, t: &Tokens) {
        let summary = self.report.as_ref().map(|r| {
            let good = r.parts.iter().filter(|p| p.ok && p.matches_source != Some(false)).count();
            format!("{good} de {} coinciden con el original", r.parts.len())
        });
        let mut clicked: Option<String> = None;
        let selected = self.selected.clone();
        let rows = &self.rows;
        egui::Frame::new()
            .fill(t.dialog_body)
            .stroke(Stroke::new(1.0, t.dialog_border))
            .corner_radius(t.radius_lg)
            .inner_margin(Margin::same(t.space_16 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing.y = t.space_6;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Piezas").font(fonts::semibold(t.font_heading)));
                    if let Some(s) = &summary {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(s).font(fonts::semibold(t.font_body)).color(t.text_dim));
                        });
                    }
                });
                let widths = column_widths(ui.available_width() - 24.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0); // the rows' inset
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for ((title, _, numeric), w) in COLUMNS.iter().zip(&widths) {
                        cell(ui, *w, *numeric, RichText::new(*title).font(fonts::semibold(t.font_caption)).color(t.dialog_label));
                    }
                });
                rule(ui, t);
                let notes_h = 40.0;
                egui::ScrollArea::vertical().max_height((ui.available_height() - notes_h).max(60.0)).auto_shrink([false, false]).show(
                    ui,
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if rows.is_empty() {
                            ui.add_space(t.space_12);
                            ui.label(RichText::new("Elige un fichero: aquí aparecen sus piezas.").size(t.font_body).color(t.text_dim));
                        }
                        for row in rows {
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
                            let is_sel = selected.as_deref() == Some(row.stem.as_str());
                            if is_sel {
                                ui.painter().rect_filled(rect, t.radius_md, t.selected_bg);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, t.radius_md, t.hover);
                            }
                            if resp.clicked() {
                                clicked = Some(row.stem.clone());
                            }
                            let mut child = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect.shrink2(Vec2::new(12.0, 0.0)))
                                    .layout(Layout::left_to_right(Align::Center)),
                            );
                            child.spacing_mut().item_spacing.x = 0.0;
                            draw_row(&mut child, t, row, &widths);
                        }
                    },
                );
                rule(ui, t);
                let note = match self.selected.as_deref() {
                    None => "Selecciona una pieza para ver sus avisos.".to_owned(),
                    Some(stem) => match self.rows.iter().find(|r| r.stem == stem).and_then(|r| r.part.as_ref()) {
                        None => format!("{stem}: todavía no se ha convertido."),
                        Some(p) if p.warnings.is_empty() => format!("{}: sin avisos.", p.stem),
                        Some(p) => format!("{}: {}", p.stem, p.warnings.join("; ")),
                    },
                };
                ui.add(egui::Label::new(RichText::new(note).size(t.font_body).color(t.text_dim)).wrap());
            });
        if let Some(c) = clicked {
            self.selected = Some(c);
        }
    }

    fn details(&mut self, ui: &mut Ui, t: &Tokens) {
        let id = egui::Id::new("details_open");
        let mut open = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
        egui::Frame::new()
            .fill(t.dialog_body)
            .stroke(Stroke::new(1.0, t.dialog_border))
            .corner_radius(t.radius_lg)
            .inner_margin(Margin::symmetric(t.space_16 as i8, t.space_8 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
                let arrow = if open { "⏷" } else { "⏵" };
                ui.painter().text(
                    rect.left_center(),
                    Align2::LEFT_CENTER,
                    format!("{arrow}   Detalles técnicos"),
                    fonts::semibold(t.font_heading),
                    t.text,
                );
                if resp.clicked() {
                    open = !open;
                }
                if open {
                    egui::ScrollArea::vertical().max_height(160.0).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&self.log).monospace().size(t.font_body)).selectable(true).wrap());
                    });
                }
            });
        ui.ctx().data_mut(|d| d.insert_temp(id, open));
    }

    fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some(toast) = &self.toast else { return };
        if toast.since.elapsed() > Duration::from_secs(6) {
            self.toast = None;
            return;
        }
        let t = self.tokens;
        let (ink, glyph) = match toast.tone {
            Tone::Good => (t.form_success, "✔"),
            Tone::Warn => (t.warning, "⚠"),
            Tone::Bad => (t.error, "✖"),
        };
        let text = toast.text.clone();
        let mut close = false;
        egui::Area::new(egui::Id::new("toast")).anchor(Align2::CENTER_BOTTOM, [0.0, -24.0]).show(ctx, |ui| {
            egui::Frame::new()
                .fill(t.dialog_body)
                .stroke(Stroke::new(1.0, t.dialog_border))
                .corner_radius(t.radius_lg)
                .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(60) })
                .inner_margin(Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_max_width(560.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(glyph).color(ink).size(16.0));
                        ui.add(egui::Label::new(RichText::new(text).size(t.font_label)).wrap());
                        if ui.add(egui::Button::new(RichText::new("×").size(16.0).color(t.text_dim)).frame(false)).clicked() {
                            close = true;
                        }
                    });
                });
        });
        if close {
            self.toast = None;
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn screenshot(&mut self, ui: &Ui) {
        let Some(shot) = &mut self.shot else { return };
        let ctx = ui.ctx();
        let waiting = std::env::var_os("CAD2SC_AUTOSTART").is_some() && (self.running || self.report.is_none());
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let [w, h] = image.size;
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, rgba) {
                let _ = buf.save(&shot.path);
            }
            if shot.close {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            self.shot = None;
            return;
        }
        if waiting {
            ctx.request_repaint_after(Duration::from_millis(200));
            return;
        }
        shot.frames += 1;
        if shot.frames > 8 && !shot.asked {
            shot.asked = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint();
    }
}

fn shell_open(path: &Path) {
    let mut cmd = std::process::Command::new("explorer.exe");
    cmd.arg(path);
    let _ = cmd.spawn();
}

/// The conversion, on its own thread.
fn work(src: PathBuf, out: PathBuf, up_y: bool, tx: Sender<Msg>, ctx: egui::Context) {
    let (tx_log, ctx_log) = (tx.clone(), ctx.clone());
    let (tx_prog, ctx_prog) = (tx.clone(), ctx.clone());
    let hooks = Hooks {
        log: Box::new(move |l| {
            let _ = tx_log.send(Msg::Log(l.to_owned()));
            ctx_log.request_repaint();
        }),
        progress: Some(Box::new(move |stage, done, total, part| {
            let _ = tx_prog.send(Msg::Progress(stage, done, total, part.cloned()));
            ctx_prog.request_repaint();
        })),
    };
    let opts = Options { up_y, ..Options::default() };
    // shown to the user, never a crash of the window
    let msg = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| converter::convert(&src, &out, &opts, &hooks))) {
        Ok(Ok(report)) => Msg::Done(report),
        Ok(Err(e)) => Msg::Fail(format!("{e:#}")),
        Err(_) => Msg::Fail("error interno del conversor".into()),
    };
    let _ = tx.send(msg);
    ctx.request_repaint();
}

// ---------------------------------------------------------------------- widgets

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Primary,
    Secondary,
}

/// A SolidCraft dialog action: accent fill for the main one, control fill and border otherwise.
fn button(ui: &mut Ui, t: &Tokens, text: &str, kind: Kind, enabled: bool, size: Option<Vec2>) -> egui::Response {
    let font = if kind == Kind::Primary { fonts::semibold(t.font_subtitle) } else { egui::FontId::proportional(t.font_button) };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    let size = size.unwrap_or(Vec2::new(galley.size().x + 2.0 * t.space_12, t.control_height + 2.0));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hovered = enabled && resp.hovered();
    let pressed = enabled && resp.is_pointer_button_down_on();
    let (fill, stroke, ink) = match kind {
        Kind::Primary if !enabled => (t.accent.gamma_multiply(0.45), Stroke::NONE, t.on_accent.gamma_multiply(0.8)),
        Kind::Primary => {
            let fill = if pressed {
                t.selected_ink
            } else if hovered {
                t.accent.gamma_multiply(0.88)
            } else {
                t.accent
            };
            (fill, Stroke::NONE, t.on_accent)
        }
        Kind::Secondary if !enabled => (t.control_fill, Stroke::new(1.0, t.dialog_rule), t.text_dim.gamma_multiply(0.7)),
        Kind::Secondary => {
            let fill = if pressed {
                t.accent_soft
            } else if hovered {
                t.hover
            } else {
                t.control_fill
            };
            (fill, Stroke::new(1.0, if hovered { t.accent } else { t.control_border }), t.text)
        }
    };
    let radius = CornerRadius::same(t.control_radius);
    ui.painter().rect_filled(rect, radius, fill);
    if stroke != Stroke::NONE {
        ui.painter().rect_stroke(rect, radius, stroke, StrokeKind::Inside);
    }
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// A card as SolidCraft draws its dialogs: body, border, and a numbered header.
fn card(ui: &mut Ui, t: &Tokens, header: Option<(&str, &str)>, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(t.dialog_body)
        .stroke(Stroke::new(1.0, t.dialog_border))
        .corner_radius(t.radius_lg)
        .inner_margin(Margin::same(t.space_16 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // top to bottom even inside a row of cards
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = t.space_8;
                if let Some((n, title)) = header {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                        ui.painter().circle_filled(rect.center(), 11.0, t.accent);
                        ui.painter().text(rect.center(), Align2::CENTER_CENTER, n, fonts::bold(t.font_body), t.on_accent);
                        ui.add_space(t.space_4);
                        ui.label(RichText::new(title).font(fonts::semibold(t.font_heading)));
                    });
                }
                add(ui);
            });
        });
}

/// SolidCraft draws its options as checkboxes; this is the same box, larger, with the label.
fn toggle(ui: &mut Ui, t: &Tokens, on: &mut bool, label: &str) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    let track = Rect::from_min_size(rect.left_center() - Vec2::new(0.0, 10.0), Vec2::new(36.0, 20.0));
    let fill = if *on { t.accent } else { t.control_border };
    ui.painter().rect_filled(track, 10, fill);
    let knob_x = if *on { track.right() - 10.0 } else { track.left() + 10.0 };
    ui.painter().circle_filled(egui::pos2(knob_x, track.center().y), 7.0, Color32::WHITE);
    ui.painter().text(
        track.right_center() + Vec2::new(10.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(t.font_label),
        t.text,
    );
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        *on = !*on;
    }
}

fn progress_bar(ui: &mut Ui, t: &Tokens, value: Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
    ui.painter().rect_filled(rect, 3, t.panel_header);
    match value {
        Some(v) => {
            let mut r = rect;
            r.set_width(rect.width() * v.clamp(0.0, 1.0));
            if v > 0.0 {
                ui.painter().rect_filled(r, 3, t.accent);
            }
        }
        None => {
            let time = ui.input(|i| i.time) as f32;
            let w = rect.width() * 0.3;
            let x = rect.left() + ((time * 0.6).fract() * (rect.width() + w)) - w;
            let r = Rect::from_min_max(egui::pos2(x.max(rect.left()), rect.top()), egui::pos2((x + w).min(rect.right()), rect.bottom()));
            ui.painter().rect_filled(r, 3, t.accent);
            ui.ctx().request_repaint();
        }
    }
}

fn rule(ui: &mut Ui, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0, t.dialog_rule);
}

fn column_widths(total: f32) -> Vec<f32> {
    let flex: f32 = COLUMNS.iter().map(|c| c.1).sum();
    COLUMNS.iter().map(|c| total * c.1 / flex).collect()
}

/// Space at each side of a table cell, so a right-aligned figure and the text next to it
/// never touch.
const CELL_PAD: f32 = 10.0;

fn cell(ui: &mut Ui, width: f32, numeric: bool, text: RichText) {
    let layout = if numeric { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
    let mut inner = cell_ui(ui, width, 22.0, layout);
    inner.add(egui::Label::new(text).truncate());
}

/// The cell's own rectangle in the row, less its padding, to draw in.
fn cell_ui(ui: &mut Ui, width: f32, height: f32, layout: Layout) -> Ui {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(CELL_PAD, 0.0))).layout(layout));
    inner.spacing_mut().item_spacing.x = 6.0;
    inner
}

fn draw_row(ui: &mut Ui, t: &Tokens, row: &Row, widths: &[f32]) {
    // name and count
    {
        let mut ui = cell_ui(ui, widths[0], 34.0, Layout::left_to_right(Align::Center));
        let ui = &mut ui;
        ui.add(egui::Label::new(RichText::new(&row.stem).font(fonts::semibold(t.font_label))).truncate());
        if row.count > 1 {
            let text = format!("×{}", row.count);
            let galley = ui.painter().layout_no_wrap(text, fonts::semibold(t.font_small), t.selected_ink);
            let (rect, _) = ui.allocate_exact_size(galley.size() + Vec2::new(12.0, 4.0), Sense::hover());
            ui.painter().rect_filled(rect, 9, t.accent_soft);
            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.selected_ink);
        }
    }
    // state chip
    {
        let mut ui = cell_ui(ui, widths[1], 34.0, Layout::left_to_right(Align::Center));
        let ui = &mut ui;
        let (ink, fill) = row.state.colors(t);
        let text = format!("{}  {}", row.state.glyph(), row.state.label());
        let galley = ui.painter().layout_no_wrap(text, fonts::semibold(t.font_caption), ink);
        let (rect, _) = ui.allocate_exact_size(galley.size() + Vec2::new(18.0, 6.0), Sense::hover());
        ui.painter().rect_filled(rect, 11, fill);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink);
    }
    let p = row.part.as_ref();
    let cm3 = |v: Option<f64>| v.map_or("—".to_owned(), |v| format!("{:.2} cm³", v / 1000.0));
    cell(ui, widths[2], true, RichText::new(cm3(p.and_then(|p| p.volume_mm3))).size(t.font_label));
    cell(ui, widths[3], true, RichText::new(cm3(p.and_then(|p| p.source_volume_mm3))).size(t.font_label).color(t.text_dim));
    let repair = p.and_then(|p| p.repair.clone()).unwrap_or_else(|| "—".into());
    cell(ui, widths[4], false, RichText::new(repair).size(t.font_body).color(t.text_dim));
    let secs = p.map_or(String::new(), |p| format!("{:.1} s", p.seconds));
    cell(ui, widths[5], true, RichText::new(secs).size(t.font_body).color(t.text_dim));
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dark = self.dark(ctx);
        if self.applied_dark != Some(dark) {
            self.tokens = Tokens::themed(dark);
            theme::apply(ctx, &self.tokens);
            self.applied_dark = Some(dark);
        }
        self.receive(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let t = self.tokens;
        let ctx = ui.ctx().clone();
        egui::Panel::top("app_bar")
            .exact_size(52.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(t.app_bar).inner_margin(Margin::symmetric(18, 0)))
            .show(ui, |ui| self.app_bar(ui));
        egui::Panel::left("steps")
            .exact_size(380.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(t.toolbar).inner_margin(Margin { left: 22, right: 8, top: 18, bottom: 18 }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.left(ui, &ctx));
            });
        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(t.toolbar).inner_margin(Margin { left: 10, right: 22, top: 18, bottom: 18 }))
            .show(ui, |ui| self.right(ui));
        self.draw_toast(&ctx);
        self.screenshot(ui);
    }
}

/// Window options: SolidCraft's renderer, the Python window's size, the Craftify icon.
pub fn native_options() -> eframe::NativeOptions {
    let mut viewport =
        egui::ViewportBuilder::default().with_title(TITLE).with_inner_size([1320.0, 880.0]).with_min_inner_size([1120.0, 740.0]);
    if let Ok(img) = image::load_from_memory(ICON_PNG) {
        let img = img.to_rgba8();
        viewport = viewport.with_icon(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() });
    }
    eframe::NativeOptions { viewport, ..Default::default() }
}

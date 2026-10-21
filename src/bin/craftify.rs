//! The converter window. Double-click Craftify.exe; no console opens with it.
#![windows_subsystem = "windows"]

fn main() -> eframe::Result {
    eframe::run_native(craftify::gui::TITLE, craftify::gui::native_options(), Box::new(|cc| Ok(Box::new(craftify::gui::App::new(cc)))))
}

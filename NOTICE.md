# Third-party notices

Craftify is an internal Hidroconta tool. It uses code and resources from others:

- **SolidCraft** (https://github.com/tangency-labs/solidcraft), MIT OR Apache-2.0,
  © SolidCraft contributors. The window copies the colours, sizes and style of SolidCraft 0.18
  (`crates/ui-egui/src/theme.rs`, `assets/styles/ui/palettes.rs`) and the way it loads its fonts
  (`crates/ui-egui/src/fonts.rs`): see `src/gui/theme.rs` and `src/gui/fonts.rs`. Craftify is
  not part of SolidCraft and is not published by its project.
- **IBM Plex Sans and IBM Plex Mono**, SIL Open Font License 1.1 (`assets/fonts/OFL-IBMPlex.txt`).
- **cadmpeg** (https://crates.io/crates/cadmpeg), Apache-2.0: shipped next to the program as
  `cadmpeg.exe`; it reads the parts' geometry.
- The Rust libraries in `Cargo.toml` (egui, eframe, rfd, serde, regex, flate2, zip, zstd,
  image…), each under its own MIT and/or Apache-2.0 licence.

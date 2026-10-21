# Avisos de terceros

Craftify es una herramienta interna de Hidroconta. Usa código y recursos de otros:

- **SolidCraft** (https://github.com/tangency-labs/solidcraft), MIT OR Apache-2.0,
  © SolidCraft contributors. El aspecto de la ventana copia los colores, tamaños y estilo
  de SolidCraft 0.18 (`crates/ui-egui/src/theme.rs`, `assets/styles/ui/palettes.rs`) y su
  forma de cargar las fuentes (`crates/ui-egui/src/fonts.rs`): ver `src/gui/theme.rs` y
  `src/gui/fonts.rs`. Craftify no forma parte de SolidCraft ni lo publica su proyecto.
- **IBM Plex Sans y IBM Plex Mono**, SIL Open Font License 1.1 (`OFL-IBMPlex.txt`).
- **cadmpeg** (https://crates.io/crates/cadmpeg), Apache-2.0: va al lado del programa como
  `cadmpeg.exe` y lee la geometría de las piezas.
- Las bibliotecas de Rust de `Cargo.toml` (egui, eframe, rfd, serde, regex, flate2, zip,
  image…), cada una con su licencia MIT y/o Apache-2.0.

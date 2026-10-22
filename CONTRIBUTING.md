# Contributing to Craftify

[English](#english) · [Español](#español)

---

## English

Thanks for wanting to help. Craftify converts SolidWorks and Fusion designs into SolidCraft
designs; it runs on Windows for now. Bug reports, ideas and code are all welcome.

### Before anything else

- **Never attach confidential files.** Do not upload SolidWorks or Fusion files, STEP files,
  reports or pictures that belong to your employer or to customers, in issues, pull requests or
  commits. Use your own or freely shared models, or describe the problem in words. The
  repository's `.gitignore` keeps CAD files out of commits; keep it that way.
- **Craftify is not part of SolidCraft.** Report SolidCraft bugs to
  [SolidCraft](https://github.com/tangency-labs/solidcraft), and do not send Craftify code to
  SolidCraft: its rules forbid automating commercial CAD products in its contributions.

### How to start

1. **Small fixes** (a typo, an obvious bug with an obvious fix): open a pull request directly.
2. **Anything bigger** (a new feature, a new source CAD, a change to how parts are converted or
   checked, a new dependency): **open an issue first** and describe what you want to do, so we
   can agree on it before you spend time on it.
3. **Bug reports**: open an issue with the Craftify version, your Windows and SolidCraft
   versions, the kind of file (SolidWorks part or assembly, Fusion `.f3d` or `.f3z`), what you
   expected and what happened. The text under *Detalles técnicos* in the window, or the output
   of `craftify-cli.exe`, helps a lot. Remove anything confidential first.

Issues and pull requests can be written in **English or Spanish**.

### Setting up

You need:

- Rust (stable, via [rustup](https://rustup.rs)) and the Visual Studio 2022 Build Tools (C++).
- SolidCraft's `solidcraft-cli`, at the version Craftify ships:
  `powershell -ExecutionPolicy Bypass -File packaging\solidcraft\fetch.ps1`.
- The patched cadmpeg: `powershell -ExecutionPolicy Bypass -File packaging\cadmpeg\build.ps1`
  (about 20 minutes the first time).
- Only to build the installer: Inno Setup 6 (`winget install JRSoftware.InnoSetup`).

Then `cargo build --release`. The README explains how to run the window and the command line.

### Making a change

1. Fork the repository and create a branch from `main` with a short descriptive name
   (for example `fix-fusion-joints` or `docs-install`).
2. Keep each pull request to **one topic**. Unrelated fixes go in separate pull requests.
3. Write code, comments and commit messages in **English**. The window's text stays in Spanish.
   Follow the style of the code around your change.
4. Open the pull request against `main` (it cannot be pushed to directly) and explain what it
   changes and why. Link the issue it comes from.

### Before asking for a review

- `cargo fmt --check` passes (the style is set in `rustfmt.toml`).
- `cargo clippy --release --all-targets -- -D warnings` passes.
- `cargo test --release` passes, and new behaviour comes with tests where it can.
- **If you change how files are converted**, convert the same models before and after your
  change and compare the two output folders with
  `python examples/compare_runs.py <before> <after>`. Say in the pull request which models you
  used (not the files themselves) and what changed.
- **If you change the window**, add screenshots in the light and the dark theme. The window can
  take them itself: set `CRAFTIFY_THEME` (`light` or `dark`) and `CRAFTIFY_SCREENSHOT`
  (a `.png` path) before running it.

The maintainers listed in `.github/CODEOWNERS` review every pull request. Releases are made by
the maintainers.

### License

Craftify is licensed under MIT OR Apache-2.0. By contributing, you agree that your contribution
is licensed under the same terms, as in the Apache License, section 5.

---

## Español

Gracias por querer ayudar. Craftify convierte diseños de SolidWorks y Fusion en diseños de
SolidCraft; por ahora funciona en Windows. Se agradecen informes de errores, ideas y código.

### Antes de nada

- **Nunca adjuntes ficheros confidenciales.** No subas ficheros de SolidWorks o Fusion, STEP,
  informes ni imágenes que sean de tu empresa o de clientes, ni en issues, ni en pull requests,
  ni en commits. Usa modelos tuyos o de libre distribución, o describe el problema con palabras.
  El `.gitignore` del repositorio deja fuera los ficheros CAD; no lo cambies para eso.
- **Craftify no forma parte de SolidCraft.** Los errores de SolidCraft se comunican en
  [SolidCraft](https://github.com/tangency-labs/solidcraft), y el código de Craftify no se envía
  a SolidCraft: sus normas prohíben automatizar productos CAD comerciales en sus contribuciones.

### Cómo empezar

1. **Arreglos pequeños** (una errata, un error evidente con un arreglo evidente): abre un pull
   request directamente.
2. **Cualquier cosa mayor** (una función nueva, un CAD de origen nuevo, un cambio en cómo se
   convierten o comprueban las piezas, una dependencia nueva): **abre antes un issue** y cuenta
   lo que quieres hacer, para ponernos de acuerdo antes de que le dediques tiempo.
3. **Informes de errores**: abre un issue con la versión de Craftify, tus versiones de Windows y
   de SolidCraft, el tipo de fichero (pieza o ensamblaje de SolidWorks, `.f3d` o `.f3z` de
   Fusion), qué esperabas y qué ha pasado. Ayuda mucho el texto de *Detalles técnicos* de la
   ventana, o lo que escribe `craftify-cli.exe`. Quita antes lo que sea confidencial.

Los issues y pull requests se pueden escribir en **inglés o en español**.

### Preparar el entorno

Necesitas:

- Rust (stable, con [rustup](https://rustup.rs)) y las Build Tools de Visual Studio 2022 (C++).
- El `solidcraft-cli` de SolidCraft, en la versión que lleva Craftify:
  `powershell -ExecutionPolicy Bypass -File packaging\solidcraft\fetch.ps1`.
- El cadmpeg parcheado: `powershell -ExecutionPolicy Bypass -File packaging\cadmpeg\build.ps1`
  (unos 20 minutos la primera vez).
- Solo para generar el instalador: Inno Setup 6 (`winget install JRSoftware.InnoSetup`).

Después, `cargo build --release`. El README explica cómo abrir la ventana y usar la línea de
comandos.

### Hacer un cambio

1. Haz un fork del repositorio y crea una rama a partir de `main` con un nombre corto y claro
   (por ejemplo `fix-fusion-joints` o `docs-install`).
2. Cada pull request trata **un solo tema**. Los arreglos que no tengan relación van en pull
   requests distintos.
3. Escribe el código, los comentarios y los mensajes de commit en **inglés**. Los textos de la
   ventana siguen en español. Sigue el estilo del código que rodea tu cambio.
4. Abre el pull request contra `main` (no se puede subir directamente a `main`) y explica qué
   cambia y por qué. Enlaza el issue del que sale.

### Antes de pedir revisión

- `cargo fmt --check` pasa (el estilo está en `rustfmt.toml`).
- `cargo clippy --release --all-targets -- -D warnings` pasa.
- `cargo test --release` pasa, y lo nuevo trae pruebas cuando se pueda.
- **Si cambias cómo se convierten los ficheros**, convierte los mismos modelos antes y después
  del cambio y compara las dos carpetas de salida con
  `python examples/compare_runs.py <antes> <después>`. Indica en el pull request qué modelos
  usaste (no los ficheros) y qué ha cambiado.
- **Si cambias la ventana**, añade capturas en tema claro y en tema oscuro. La propia ventana
  puede sacarlas: define `CRAFTIFY_THEME` (`light` o `dark`) y `CRAFTIFY_SCREENSHOT` (la ruta de
  un `.png`) antes de abrirla.

Los responsables que figuran en `.github/CODEOWNERS` revisan cada pull request. Las versiones
las publican los responsables.

### Licencia

Craftify se distribuye bajo MIT OR Apache-2.0. Al contribuir, aceptas que tu contribución se
distribuya en esas mismas condiciones, como dice la sección 5 de la licencia Apache.

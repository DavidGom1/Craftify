# Craftify

Craftify converts SolidWorks assemblies and parts, and Fusion designs, into
[SolidCraft](https://github.com/tangency-labs/solidcraft) designs, and checks every part against
the original. It is written in Rust, with a window in **egui** (the same library SolidCraft
uses) and SolidCraft's look: its colours, sizes, IBM Plex typeface and light and dark themes,
copied from SolidCraft 0.18 (see `NOTICE.md`).

An internal Hidroconta tool. **It is not part of SolidCraft and is not contributed to its
repository**: SolidCraft's rules forbid automating or extracting data from commercial CAD
products in its contributions.

The first version of Craftify was written in Python (package `cad2solidcraft`); it remains in
the git history up to commit `e346a53`. This version does the same and gives the same results
(see "Tests").

The window and the messages it shows are in Spanish, for the people who use it.

## What it does

- **SolidWorks** (`.SLDASM`, `.SLDPRT`): every part becomes its own design, with its exact
  geometry (a STEP file from cadmpeg) repaired against the display mesh SolidWorks saves in the
  file, and turned upright (SolidWorks uses Y as up; SolidCraft, Z). The assembly links those
  designs, one occurrence per instance. Each part is compared with that mesh by volume and
  bounding box.
- **Fusion** (`.f3d`, `.f3z`), without Fusion installed: the components, occurrences and bodies
  are read from the file, cadmpeg writes the geometry, and the STEP file is split per component
  and tidied the way Fusion's own export writes it (seams, trimmed curves, fans of faces round a
  vertex, model tolerance). In a `.f3z`, external references are converted from their own
  document. Each part is compared with the boxes Fusion caches for its bodies.
- It keeps the original's preview picture, a picture of the result and a `report.json`.

## Code

| Folder | What it holds |
|---|---|
| `src/sw/` | the SolidWorks container, assemblies, the mesh from `cadmpeg dump`, the conversion |
| `src/step/` | the STEP reader, orientation repairs (`fix`, `mesh_fix`), splitting by body (`split`) and Fusion-style tidying (`clean`, `edit`) |
| `src/fusion/` | the `.f3d` reader (`f3d`), conversion without Fusion (`offline`), previews, and the older add-in route (`bridge`) |
| `src/solidcraft/` | `solidcraft-cli` scripts, placements and assemblies |
| `src/mesh.rs` | volume, bounding box and distance to a mesh |
| `src/pyset.rs` | sets that iterate in CPython's order (so the ids written match the Python version's) |
| `src/gui/` | the window and SolidCraft's theme |
| `fusion_addin/` | Fusion add-in (Python, runs inside Fusion): the older route, kept only to compare results |
| `packaging/cadmpeg/` | the cadmpeg patch for `.f3d` files with external references, and `build.ps1`, which builds it |

## Requirements

| Program | Used for |
|---|---|
| Rust (rustup, stable) and VS 2022 Build Tools | building |
| SolidCraft 0.16 or later | its `solidcraft-cli` builds the designs (`winget install bherbruck.SolidCraft`) |
| patched cadmpeg | `packaging/cadmpeg/bin/cadmpeg.exe`; rebuild it with `powershell -ExecutionPolicy Bypass -File packaging\cadmpeg\build.ps1` |

## Usage

```bash
cargo build --release
```

- Window: `target/release/craftify.exe`.
- Command line: `target/release/craftify-cli.exe "C:/path/ASSEMBLY.SLDASM" --out "C:/path/output"`
  (`--up z` keeps SolidWorks parts as they are, `--no-picture` skips the picture of the result).
- To hand it out: `package.ps1` puts `Craftify.exe`, `craftify-cli.exe` and `cadmpeg.exe` in
  `dist/Craftify` (about 93 MB). The computers it runs on need only SolidCraft.

The window keeps its choices in `%APPDATA%\cad2solidcraft\settings.json` and reads variables for
automated checks: `CAD2SC_SOURCE`, `CAD2SC_OUT`, `CAD2SC_AUTOSTART`, `CAD2SC_DONE_FILE`, and
`CRAFTIFY_THEME`, `CRAFTIFY_SCREENSHOT` and `CRAFTIFY_CLOSE` to save a picture of the window.

SolidWorks and Fusion files are company data: they are never committed (`.gitignore`). The
Fusion test files live in `out/test-data/` (ignored).

## Tests

```bash
cargo test --release
```

Checked against the Python version before it was retired:

- SolidWorks (an assembly of 15 parts and 18 instances): the 6 STEP repairs, the rule-based one
  and the upright turn give the same text byte for byte on all 15 parts (120 files); a whole
  conversion gives the same `report.json` (15 of 15 parts match SolidWorks) in about 50 s
  instead of 114 s.
- Fusion (`LTC 10Kg Tool.f3d`, `LTC 10Kg.f3d`, `Dobot CR5A.f3z`, 15 parts): the same reports,
  the same STEP files byte for byte (20 files) and the same SolidCraft scripts; the Dobot in
  707 s instead of 1042 s.

`examples/compare_runs.py` compares two output folders (for instance before and after a change);
`examples/golden.rs` applies the STEP repairs to a set of parts; `examples/pyset_check.*` checks
the order of `src/pyset.rs` against CPython.

## Decisions

- **cadmpeg runs as a separate program**, not linked as a library: the STEP file is then exactly
  what `cadmpeg convert` writes, with its warnings, and the patch is applied by building cadmpeg.
- **`solidcraft-cli` runs as a separate program**: SolidCraft does not publish its engine as a
  library, and linking it would mean building its whole kernel.
- The window does not use SolidCraft's UI crate (it is not published and pulls in the engine):
  it copies its theme.

## Not done yet

- The timeline (editable features): the Python version had an unconnected translator; cadmpeg
  0.6 decodes too little of SolidWorks sketches.
- Subassemblies, mates, and configurations other than the active one.

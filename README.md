# Craftify

Craftify convierte ensamblajes y piezas de SolidWorks, y diseños de Fusion, en diseños de
[SolidCraft](https://github.com/tangency-labs/solidcraft), y comprueba cada pieza contra el
original. Está escrito en Rust, con una ventana en **egui** (la misma biblioteca que SolidCraft)
y el aspecto de SolidCraft: colores, tamaños, tipografía IBM Plex y modo claro y oscuro copiados
de SolidCraft 0.18 (ver `NOTICE.md`).

Herramienta interna de Hidroconta. **No forma parte de SolidCraft ni se envía a su
repositorio**: las reglas de SolidCraft prohíben automatizar o extraer datos de productos CAD
comerciales en sus contribuciones.

La primera versión de Craftify estaba en Python (paquete `cad2solidcraft`); sigue en el
historial de git hasta el commit `e346a53`. Esta versión hace lo mismo y da los mismos
resultados (ver «Comprobado contra Python»).

## Qué hace

- **SolidWorks** (`.SLDASM`, `.SLDPRT`): convierte cada pieza en su propio diseño, con su
  geometría exacta (STEP de cadmpeg) corregida con la malla que el propio SolidWorks guarda en
  el fichero, y de pie (SolidWorks usa Y como vertical; SolidCraft, Z). El ensamblaje enlaza esos
  diseños, una ocurrencia por instancia. Cada pieza se compara en volumen y caja con esa malla.
- **Fusion** (`.f3d`, `.f3z`), sin tener Fusion: lee los componentes, ocurrencias y cuerpos del
  fichero, cadmpeg escribe la geometría, y el STEP se separa por componente y se limpia como lo
  exporta Fusion (costuras, curvas recortadas, abanicos de caras, tolerancia). En un `.f3z`, las
  referencias externas se convierten desde su propio documento. Cada pieza se compara con las
  cajas que guarda Fusion.
- Deja la vista previa del original, una imagen del resultado y un `report.json`.

## Código

| Carpeta | Qué hace |
|---|---|
| `src/sw/` | contenedor de SolidWorks, ensamblajes, malla de `cadmpeg dump`, conversión |
| `src/step/` | lector de STEP, correcciones de orientación (`fix`, `mesh_fix`), separación por cuerpos (`split`) y limpieza al estilo de Fusion (`clean`, `edit`) |
| `src/fusion/` | lector de `.f3d` (`f3d`), conversión sin Fusion (`offline`), vista previa, y la vía antigua del complemento (`bridge`) |
| `src/solidcraft/` | scripts de `solidcraft-cli`, colocaciones y ensamblajes |
| `src/mesh.rs` | volumen, caja y distancia a una malla |
| `src/pyset.rs` | conjuntos que se recorren en el orden de CPython (para dar los mismos ids que la versión Python) |
| `src/gui/` | la ventana y el tema de SolidCraft |
| `fusion_addin/` | complemento de Fusion (Python, corre dentro de Fusion): la vía antigua, solo para comparar |
| `packaging/cadmpeg/` | parche de cadmpeg para los `.f3d` con referencias externas, y `build.ps1` que lo compila |

## Requisitos

| Programa | Para qué |
|---|---|
| Rust (rustup, stable) y VS 2022 Build Tools | compilar |
| SolidCraft 0.16 o posterior | su `solidcraft-cli` construye los diseños (`winget install bherbruck.SolidCraft`) |
| cadmpeg parcheado | `packaging/cadmpeg/bin/cadmpeg.exe`; se rehace con `powershell -ExecutionPolicy Bypass -File packaging\cadmpeg\build.ps1` |

## Uso

```bash
cargo build --release
```

- Ventana: `target/release/craftify.exe`.
- Línea de comandos: `target/release/craftify-cli.exe "C:/ruta/ENSAMBLAJE.SLDASM" --out "C:/ruta/salida"`
  (`--up z` deja las piezas de SolidWorks sin girar, `--no-picture` no dibuja el resultado).
- Para repartir: `package.ps1` deja en `dist/Craftify` `Craftify.exe`, `craftify-cli.exe` y
  `cadmpeg.exe` (unos 93 MB). Los equipos donde se use solo necesitan SolidCraft.

La ventana guarda sus opciones en `%APPDATA%\cad2solidcraft\settings.json` y admite variables
para pruebas automáticas: `CAD2SC_SOURCE`, `CAD2SC_OUT`, `CAD2SC_AUTOSTART`,
`CAD2SC_DONE_FILE`, y `CRAFTIFY_THEME`, `CRAFTIFY_SCREENSHOT` y `CRAFTIFY_CLOSE` para sacar una
captura de la ventana.

Los ficheros de SolidWorks y Fusion son datos de la empresa: no se suben al repositorio
(`.gitignore`). Los de Fusion de prueba están en `out/test-data/` (ignorado).

## Pruebas

```bash
cargo test --release
```

Comprobado contra Python, antes de retirarlo:

- SolidWorks (ensamblaje de 15 piezas y 18 instancias): las 6 correcciones de STEP, las reglas y
  el giro de las 15 piezas dan el mismo texto byte a byte (120 ficheros); la conversión entera da
  el mismo `report.json` (15 de 15 coinciden con SolidWorks) en unos 50 s frente a 114 s.
- Fusion (`LTC 10Kg Tool.f3d`, `LTC 10Kg.f3d`, `Dobot CR5A.f3z`, 15 piezas): mismos informes,
  mismos STEP byte a byte (20 ficheros) y mismos scripts; el Dobot en 707 s frente a 1042 s.

`examples/compare_runs.py` compara dos carpetas de salida (por ejemplo, antes y después de un
cambio); `examples/golden.rs` aplica las correcciones de STEP a un juego de piezas;
`examples/pyset_check.*` comprueba el orden de `src/pyset.rs` contra CPython.

## Decisiones

- **cadmpeg va como programa aparte**, no enlazado como biblioteca: así el STEP es exactamente
  el que da `cadmpeg convert`, con sus avisos, y el parche se aplica compilando cadmpeg.
- **`solidcraft-cli` va como programa aparte**: SolidCraft no publica su motor como biblioteca y
  enlazarlo costaría compilar su núcleo entero.
- La ventana no usa el crate de interfaz de SolidCraft (no está publicado y arrastra el motor):
  copia su tema.

## Lo que falta

- La línea temporal (operaciones editables): la versión Python tenía un traductor
  desconectado; cadmpeg 0.6 decodifica demasiado poco de los croquis de SolidWorks.
- Subensamblajes, relaciones de posición y configuraciones distintas de la activa.

# cad2solidcraft

Convierte ensamblajes y piezas de CAD comerciales en diseños de [SolidCraft](https://github.com/tangency-labs/solidcraft).
Empieza por SolidWorks; Fusion vendrá después.

Herramienta interna de Hidroconta. **No forma parte de SolidCraft ni se envía a su repositorio**:
las reglas de SolidCraft prohíben automatizar o extraer datos de productos CAD comerciales en
sus contribuciones.

## Qué hace

A partir de un ensamblaje `.SLDASM`:

- Lee qué piezas lleva, cuántas veces, en qué posición y con qué configuración.
- Convierte cada pieza `.SLDPRT` en su propio diseño `.solidcraft`, con su geometría exacta
  (B-rep) como operación de importación, y de pie (SolidWorks usa Y como vertical; SolidCraft, Z).
- Crea el ensamblaje como un diseño `.solidcraft` cuyos componentes **enlazan** a esos diseños
  de pieza, con una ocurrencia por instancia de SolidWorks. Si cambias una pieza, el ensamblaje
  la actualiza con *Update Links*.
- **Comprueba cada pieza contra SolidWorks**: compara volumen y caja con la malla que el propio
  SolidWorks guarda en el fichero, y avisa de cualquier pieza que no coincida.
- Guarda la vista previa de SolidWorks, una imagen del resultado y un informe `report.json`.

También convierte **piezas sueltas** `.SLDPRT` en un diseño cada una.

Lo que **todavía no** hace:

- La línea temporal (operaciones editables). Hay un traductor experimental en
  `cad2solidcraft/sw/timeline.py`, desconectado: cadmpeg 0.6.2 decodifica demasiado poco de los
  croquis de SolidWorks para reconstruir las piezas con exactitud.
- Subensamblajes, relaciones de posición (se convertirán en uniones), piezas suprimidas y
  configuraciones distintas de la activa.

## Requisitos

| Programa | Para qué | Cómo instalarlo |
|---|---|---|
| Python 3.11 o posterior | el conversor | python.org o Microsoft Store |
| Flet 1.0 | la ventana | `python -m pip install --user -r requirements.txt` |
| SolidCraft 0.16 o posterior | su `solidcraft-cli` construye los diseños | `winget install bherbruck.SolidCraft` |
| cadmpeg 0.6.2 | lee la geometría de las piezas de SolidWorks | `cargo install cadmpeg --version 0.6.2 --locked` (necesita Rust) |

No hace falta tener SolidWorks instalado. Sin Flet, la ventana se abre en su versión sencilla
(Tkinter).

## Uso

Con ventana: doble clic en **`Convertir a SolidCraft.pyw`**. Elige un ensamblaje o una pieza
(su vista previa aparece al momento), la carpeta de salida, y pulsa **Convertir**. La tabla
muestra cada pieza según termina, con su volumen frente al de SolidWorks; al seleccionar una
pieza se ven sus avisos. Al terminar, **Abrir en SolidCraft** abre el resultado.

Por línea de comandos:

```bash
python -m cad2solidcraft "C:/ruta/al/ENSAMBLAJE.SLDASM" --out "C:/ruta/de/salida"
```

```bash
python -m cad2solidcraft "C:/ruta/a/la/PIEZA.SLDPRT" --out "C:/ruta/de/salida"
```

Las piezas de un ensamblaje se buscan como lo hace SolidWorks: primero en la carpeta del
ensamblaje, después en las rutas que el ensamblaje recuerda y por último en las subcarpetas.

Resultado de un ensamblaje, dentro de la carpeta de salida:

```
<ENSAMBLAJE>/
  <ENSAMBLAJE>.solidcraft      el ensamblaje (abre este)
  parts/<pieza>.solidcraft     un diseño por pieza
  preview_solidworks.png       vista previa guardada por SolidWorks
  preview_solidcraft.png       el resultado, dibujado por SolidCraft
  report.json                  piezas, avisos, volúmenes y tiempos
  build/                       ficheros intermedios (STEP y scripts)
```

Una pieza suelta deja `<PIEZA>/<PIEZA>.solidcraft` con las mismas imágenes e informe.

## Cómo funciona

- `cad2solidcraft/sw/container.py` lee el contenedor de bloques de SolidWorks 2015+.
- `cad2solidcraft/sw/assembly.py` saca las instancias del bloque `Contents/Config-0`.
- `cad2solidcraft/sw/ir.py` lee el modelo que da `cadmpeg dump`, incluida la malla de
  visualización que guardó SolidWorks, que sirve de referencia.
- `cad2solidcraft/step_fix.py` y `step_mesh_fix.py` corrigen las marcas de orientación de caras
  y aristas de los STEP que escribe cadmpeg 0.6.2. Cada pieza prueba varias correcciones y se
  queda con la que coincide con la malla de SolidWorks. Sin esto, las piezas se ven
  translúcidas, salen discos tapando agujeros y los volúmenes son erróneos.
- `cad2solidcraft/sw/convert.py` lo une todo y genera scripts de comandos de SolidCraft.
- `cad2solidcraft/app.py` es la ventana (Flet); `gui.py`, la de respaldo (Tkinter).

Las piezas se giran de Y a Z dentro del STEP y no con la opción `up` de SolidCraft: en
SolidCraft 0.16.2 una importación girada se reconstruye distinta al guardar y reabrir.

El formato de SolidWorks no es público: los lectores se basan en el análisis de nuestros propios
ficheros, para interoperar con ellos. Las piezas de otros años de SolidWorks pueden necesitar
ajustes; el informe dice qué no se ha podido leer.

## Pruebas

```bash
python -m unittest discover -s tests -t .
```

Los ficheros de SolidWorks son datos de la empresa: no se suben al repositorio (`.gitignore`).

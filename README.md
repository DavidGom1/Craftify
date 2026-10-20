# cad2solidcraft

Convierte proyectos de CAD comerciales en diseños de [SolidCraft](https://github.com/tangency-labs/solidcraft).
Empieza por SolidWorks; Fusion vendrá después.

Herramienta interna de Hidroconta. **No forma parte de SolidCraft ni se envía a su repositorio**:
las reglas de SolidCraft prohíben automatizar o extraer datos de productos CAD comerciales en
sus contribuciones.

## Qué hace (fase 1)

A partir de un ensamblaje `.SLDASM`:

- Lee qué piezas lleva, cuántas veces, en qué posición y con qué configuración.
- Convierte cada pieza `.SLDPRT` en su propio diseño `.solidcraft`, con su geometría exacta
  (B-rep) como operación de importación, y de pie (SolidWorks usa Y como vertical; SolidCraft, Z).
- Crea el ensamblaje como un diseño `.solidcraft` cuyos componentes **enlazan** a esos diseños
  de pieza, con una ocurrencia por instancia de SolidWorks. Si cambias una pieza, el ensamblaje
  la actualiza con *Update Links*.
- Guarda la vista previa de SolidWorks y una imagen del resultado para compararlas, y un
  informe `report.json`.

Lo que **todavía no** hace:

- La línea temporal (operaciones editables de cada pieza): es la fase 2.
- Subensamblajes, relaciones de posición (se convertirán en uniones), piezas suprimidas y
  configuraciones distintas de la activa.

## Requisitos

| Programa | Para qué | Cómo instalarlo |
|---|---|---|
| Python 3.11 o posterior | el conversor y su ventana | python.org o Microsoft Store |
| SolidCraft 0.16 o posterior | su `solidcraft-cli` construye los diseños | `winget install bherbruck.SolidCraft` |
| cadmpeg 0.6.2 | lee la geometría de las piezas de SolidWorks | `cargo install cadmpeg --version 0.6.2 --locked` (necesita Rust) |

No hace falta tener SolidWorks instalado.

## Uso

Con ventana: doble clic en **`Convertir a SolidCraft.pyw`**, elige el ensamblaje y la carpeta
de salida y pulsa **Convertir**. Al terminar, **Abrir en SolidCraft** abre el ensamblaje.

Por línea de comandos:

```bash
python -m cad2solidcraft "C:/ruta/al/ENSAMBLAJE.SLDASM" --out "C:/ruta/de/salida"
```

Las piezas se buscan como lo hace SolidWorks: primero en la carpeta del ensamblaje, después en
las rutas que el ensamblaje recuerda y por último en las subcarpetas.

Resultado, dentro de la carpeta de salida:

```
<ENSAMBLAJE>/
  <ENSAMBLAJE>.solidcraft      el ensamblaje (abre este)
  parts/<pieza>.solidcraft     un diseño por pieza
  preview_solidworks.png       vista previa guardada por SolidWorks
  preview_solidcraft.png       el resultado, dibujado por SolidCraft
  report.json                  piezas, avisos, volúmenes y tiempos
  build/                       ficheros intermedios (STEP y scripts)
```

## Cómo funciona

- `cad2solidcraft/sw/container.py` lee el contenedor de bloques de SolidWorks 2015+.
- `cad2solidcraft/sw/assembly.py` saca las instancias del bloque `Contents/Config-0`.
- `cad2solidcraft/step_fix.py` corrige la orientación de caras y aristas circulares de los STEP
  que escribe cadmpeg 0.6.2, que marca todas las caras como invertidas. Sin esta corrección las
  piezas se ven translúcidas, los agujeros salen como pivotes y los volúmenes son erróneos.
- `cad2solidcraft/sw/convert.py` lo une todo y genera scripts de comandos de SolidCraft.
- `cad2solidcraft/gui.py` es la ventana (Tkinter, incluido en Python).

El formato de SolidWorks no es público: los lectores se basan en el análisis de nuestros propios
ficheros, para interoperar con ellos. Las piezas de otros años de SolidWorks pueden necesitar
ajustes; el informe dice qué no se ha podido leer.

## Pruebas

```bash
python -m unittest discover -s tests -t .
```

Los ficheros de SolidWorks son datos de la empresa: no se suben al repositorio (`.gitignore`).

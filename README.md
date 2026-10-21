# Craftify

Craftify (paquete `cad2solidcraft`) convierte ensamblajes y piezas de CAD comerciales en diseños de [SolidCraft](https://github.com/tangency-labs/solidcraft):
SolidWorks (`.SLDASM`, `.SLDPRT`) y Fusion (`.f3d`, `.f3z`). No hace falta tener instalado
ninguno de los dos.

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

A partir de un diseño de Fusion `.f3d` o de un proyecto `.f3z` (el diseño con sus referencias
externas, como lo exporta Fusion):

- Lee los componentes, sus cuerpos y la posición de cada ocurrencia, también las que coloca una
  unión (*joint*): usa la posición que Fusion muestra.
- Las referencias externas de un `.f3z` se convierten desde su propio documento.
- Convierte cada componente con cuerpos en un diseño `.solidcraft`, y el diseño completo en un
  ensamblaje que los enlaza, como con SolidWorks. Los ejes se quedan como en Fusion.
- **Comprueba cada pieza** contra la caja que Fusion guarda para cada cuerpo y avisa de lo que
  SolidCraft no haya podido leer bien.

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
| cadmpeg 0.6 con nuestro parche | lee la geometría de SolidWorks y de Fusion | `packaging/cadmpeg/build.ps1` (necesita Rust; deja `packaging/cadmpeg/bin/cadmpeg.exe`) |

No hace falta tener SolidWorks ni Fusion instalados. El parche de `packaging/cadmpeg/` deja a
cadmpeg leer diseños con referencias externas (sin él, rechaza el diseño entero); la versión
empaquetada (`Craftify.exe`) ya lo lleva dentro.

## Uso

Con ventana: doble clic en **`Craftify.pyw`** (o en `Craftify.exe`, la versión empaquetada). Elige un ensamblaje o una pieza
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

```bash
python -m cad2solidcraft "C:/ruta/al/PROYECTO.f3z" --out "C:/ruta/de/salida"
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
- `cad2solidcraft/fusion/f3d.py` lee el segmento *Design* de un `.f3d`: componentes, ocurrencias
  con su matriz (o la que resuelve la unión), qué cuerpos tiene cada componente y su caja.
- `cad2solidcraft/fusion/offline.py` convierte un diseño de Fusion: cadmpeg escribe los cuerpos
  de cada documento en un STEP, en un orden fijo (por bloque BREP y cuerpo) que dice a qué
  componente pertenece cada uno; `step_split.py` lo parte por componente.
- `cad2solidcraft/step_clean.py` deja ese STEP como lo escribiría la exportación de Fusion:
  costuras en las caras que dan la vuelta a un cilindro o cono (en el sitio donde las pone
  Fusion), curvas recortadas a sus aristas, la cáscara exterior en su sitio y la tolerancia
  adecuada. Sin esto, SolidCraft recorta mal algunas caras y deja huecos.
- `cad2solidcraft/fusion/convert.py` y `fusion_addin/` son una vía alternativa que exporta a
  través del propio Fusion (sirve para comparar resultados; necesita Fusion abierto).
- `cad2solidcraft/app.py` es la ventana (Flet).

Las piezas se giran de Y a Z dentro del STEP y no con la opción `up` de SolidCraft: en
SolidCraft 0.16.2 una importación girada se reconstruye distinta al guardar y reabrir.

Los formatos de SolidWorks y Fusion no son públicos: los lectores se basan en el análisis de
nuestros propios ficheros y en la documentación de cadmpeg, para interoperar con ellos. Las piezas de otros años de SolidWorks pueden necesitar
ajustes; el informe dice qué no se ha podido leer.

## Pruebas

```bash
python -m unittest discover -s tests -t .
```

Los ficheros de SolidWorks y Fusion son datos de la empresa: no se suben al repositorio (`.gitignore`).

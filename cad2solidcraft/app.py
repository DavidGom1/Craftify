"""Converter window built with Flet (Flutter, Material 3).

Run with ``python -m cad2solidcraft.app`` or double-click ``Convertir a SolidCraft.pyw``.
"""

from __future__ import annotations

import os
import subprocess
from dataclasses import dataclass
from pathlib import Path

import flet as ft

from . import settings as prefs
from .settings import DEFAULT_OUT
from .sw.assembly import read_assembly
from .sw.convert import PREVIEW_RESULT, SUPPORTED, PartResult, Report, convert, preview_png

TITLE = "Convertir a SolidCraft"
SEED = ft.Colors.INDIGO
MUTED = ft.Colors.ON_SURFACE_VARIANT

STATE_STYLE = {
    "waiting": ("En espera", ft.Icons.SCHEDULE, ft.Colors.OUTLINE),
    "running": ("Convirtiendo", ft.Icons.AUTORENEW, ft.Colors.PRIMARY),
    "ok": ("Coincide", ft.Icons.CHECK_CIRCLE, ft.Colors.GREEN),
    "warn": ("No coincide", ft.Icons.WARNING_AMBER_ROUNDED, ft.Colors.AMBER),
    "error": ("Error", ft.Icons.ERROR_OUTLINE, ft.Colors.RED),
}

# Parts table: (title, flex, numeric)
COLUMNS = [("Pieza", 4, False), ("Estado", 3, False), ("Volumen", 2, True), ("En SolidWorks", 2, True),
           ("Corrección", 3, False), ("Tiempo", 1, True)]


def chip(state: str) -> ft.Control:
    text, icon, color = STATE_STYLE[state]
    return ft.Container(
        content=ft.Row([ft.Icon(icon, size=15, color=color), ft.Text(text, size=12, color=color, weight=ft.FontWeight.W_600)],
                       spacing=6, tight=True),
        bgcolor=ft.Colors.with_opacity(0.14, color),
        border_radius=20,
        padding=ft.Padding.symmetric(horizontal=10, vertical=4),
    )


def count_badge(count: int) -> ft.Control:
    return ft.Container(
        content=ft.Text(f"×{count}", size=11, weight=ft.FontWeight.W_600, color=ft.Colors.ON_SECONDARY_CONTAINER),
        bgcolor=ft.Colors.SECONDARY_CONTAINER, border_radius=10, padding=ft.Padding.symmetric(horizontal=8, vertical=1),
    )


def step_header(number: str, title: str) -> ft.Control:
    return ft.Row(
        [
            ft.Container(ft.Text(number, size=13, weight=ft.FontWeight.BOLD, color=ft.Colors.ON_PRIMARY),
                         width=26, height=26, border_radius=13, bgcolor=ft.Colors.PRIMARY, alignment=ft.Alignment.CENTER),
            ft.Text(title, size=15, weight=ft.FontWeight.W_600),
        ],
        spacing=10,
    )


def panel(*controls: ft.Control, padding: int = 18, expand: bool | int = False, spacing: int = 10) -> ft.Container:
    """A rounded surface: the app's card."""
    return ft.Container(
        content=ft.Column(list(controls), spacing=spacing, tight=not expand, expand=expand),
        padding=padding,
        border_radius=18,
        bgcolor=ft.Colors.SURFACE_CONTAINER,
        border=ft.Border.all(1, ft.Colors.with_opacity(0.06, ft.Colors.ON_SURFACE)),
        expand=expand,
    )


@dataclass
class PartRow:
    box: ft.Container
    cells: list[ft.Container]


class ConverterApp:
    def __init__(self, page: ft.Page) -> None:
        self.page = page
        self.settings = prefs.load()
        self.report: Report | None = None
        self.rows: dict[str, PartRow] = {}
        self.states: dict[str, str] = {}
        self.stems: list[str] = []
        self.selected: str | None = None
        self.running = False
        self.picker = ft.FilePicker()
        page.services.append(self.picker)

        page.title = TITLE
        page.theme = ft.Theme(color_scheme_seed=SEED)
        page.dark_theme = ft.Theme(color_scheme_seed=SEED)
        page.theme_mode = {"light": ft.ThemeMode.LIGHT, "dark": ft.ThemeMode.DARK}.get(self.settings.get("theme"), ft.ThemeMode.SYSTEM)
        page.padding = 0
        page.bgcolor = ft.Colors.SURFACE
        page.window.width, page.window.height = 1320, 880
        page.window.min_width, page.window.min_height = 1120, 740

        self.source = os.environ.get("CAD2SC_SOURCE") or self.settings.get("source", "")
        self.out = os.environ.get("CAD2SC_OUT") or self.settings.get("out", str(DEFAULT_OUT))
        self._build()
        self._show_source()
        if os.environ.get("CAD2SC_AUTOSTART"):  # for automated checks of the window
            self.start()

    # ------------------------------------------------------------------ layout
    def _build(self) -> None:
        p = self.page
        self.theme_btn = ft.IconButton(ft.Icons.DARK_MODE_OUTLINED, selected_icon=ft.Icons.LIGHT_MODE_OUTLINED,
                                       tooltip="Cambiar entre claro y oscuro", on_click=self.toggle_theme,
                                       selected=p.theme_mode == ft.ThemeMode.DARK)
        header = ft.Container(
            ft.Row(
                [
                    ft.Container(ft.Icon(ft.Icons.VIEW_IN_AR_ROUNDED, color=ft.Colors.ON_PRIMARY_CONTAINER, size=26),
                                 width=46, height=46, border_radius=14, bgcolor=ft.Colors.PRIMARY_CONTAINER, alignment=ft.Alignment.CENTER),
                    ft.Column([ft.Text(TITLE, size=21, weight=ft.FontWeight.BOLD),
                               ft.Text("Ensamblajes y piezas de SolidWorks a diseños de SolidCraft, comprobados con el original",
                                       size=12, color=MUTED)], spacing=0, tight=True, expand=True),
                    self.theme_btn,
                ],
                spacing=14,
            ),
            padding=ft.Padding.only(left=24, right=16, top=18, bottom=6),
        )

        # left column: the three steps and the actions
        self.src_name = ft.Text("Nada elegido todavía", size=16, weight=ft.FontWeight.W_600, max_lines=2,
                                overflow=ft.TextOverflow.ELLIPSIS)
        self.src_folder = ft.Text("", size=12, color=MUTED, max_lines=1, overflow=ft.TextOverflow.ELLIPSIS)
        self.src_info = ft.Text("Un ensamblaje .SLDASM, cuyas piezas se buscan en su carpeta, o una pieza .SLDPRT suelta",
                                size=12, color=MUTED)
        self.out_text = ft.Text(self.out, size=13, max_lines=2, overflow=ft.TextOverflow.ELLIPSIS, tooltip=self.out)
        self.up_switch = ft.Switch(label="Poner las piezas de pie", value=self.settings.get("up_y", True), on_change=self._remember)
        self.convert_btn = ft.FilledButton(
            content=ft.Text("Convertir", size=15, weight=ft.FontWeight.W_600), icon=ft.Icons.PLAY_ARROW_ROUNDED,
            on_click=self.start, expand=True, height=52,
            style=ft.ButtonStyle(shape=ft.RoundedRectangleBorder(radius=14)),
        )
        self.progress = ft.ProgressBar(value=0, bar_height=6, border_radius=3)
        self.status = ft.Text("Elige un fichero para empezar.", size=13, color=MUTED)
        self.open_btn = ft.FilledTonalButton(content="Abrir en SolidCraft", icon=ft.Icons.OPEN_IN_NEW, on_click=self.open_result,
                                             disabled=True, expand=True)
        self.folder_btn = ft.OutlinedButton(content="Abrir carpeta", icon=ft.Icons.FOLDER_OPEN_OUTLINED, on_click=self.open_folder,
                                            disabled=True, expand=True)

        left = ft.Container(
            ft.Column(
                [
                    panel(step_header("1", "Ensamblaje o pieza de SolidWorks"), self.src_name, self.src_folder, self.src_info,
                          ft.FilledTonalButton(content="Elegir fichero", icon=ft.Icons.FILE_OPEN_OUTLINED, on_click=self.pick_source)),
                    panel(step_header("2", "Carpeta de salida"), self.out_text,
                          ft.OutlinedButton(content="Cambiar carpeta", icon=ft.Icons.DRIVE_FOLDER_UPLOAD_OUTLINED, on_click=self.pick_out)),
                    panel(step_header("3", "Opciones"), self.up_switch,
                          ft.Text("SolidWorks usa Y como vertical y SolidCraft usa Z.", size=12, color=MUTED)),
                    ft.Container(height=2),
                    ft.Row([self.convert_btn]),
                    self.progress,
                    self.status,
                    ft.Column([ft.Row([self.open_btn]), ft.Row([self.folder_btn])], spacing=8),
                ],
                spacing=12,
                scroll=ft.ScrollMode.AUTO,
            ),
            width=390,
            padding=ft.Padding.only(left=24, top=10, right=6, bottom=20),
        )

        # right column: pictures, parts and details (fixed layout; only the list scrolls)
        self.pic_src = ft.Container(self._placeholder(), alignment=ft.Alignment.CENTER, expand=True)
        self.pic_out = ft.Container(self._placeholder(), alignment=ft.Alignment.CENTER, expand=True)
        pictures = ft.Row(
            [
                panel(self._caption(ft.Icons.PHOTO_OUTLINED, "SolidWorks · vista previa guardada"), self.pic_src, padding=14, expand=True),
                panel(self._caption(ft.Icons.VIEW_IN_AR_OUTLINED, "SolidCraft · resultado"), self.pic_out, padding=14, expand=True),
            ],
            spacing=12,
            height=290,
        )
        self.summary = ft.Text("", size=13, weight=ft.FontWeight.W_600, color=MUTED)
        head = ft.Container(
            ft.Row([self._cell(ft.Text(t, size=12, weight=ft.FontWeight.W_600, color=MUTED), f, n) for t, f, n in COLUMNS], spacing=8),
            padding=ft.Padding.symmetric(horizontal=12, vertical=6),
        )
        self.list = ft.ListView(spacing=2, expand=True)
        self.notes = ft.Text("Selecciona una pieza para ver sus avisos.", size=12, color=MUTED)
        parts = panel(
            ft.Row([ft.Text("Piezas", size=15, weight=ft.FontWeight.W_600), self.summary], alignment=ft.MainAxisAlignment.SPACE_BETWEEN),
            head,
            ft.Divider(height=1, thickness=1, color=ft.Colors.with_opacity(0.08, ft.Colors.ON_SURFACE)),
            self.list,
            ft.Divider(height=1, thickness=1, color=ft.Colors.with_opacity(0.08, ft.Colors.ON_SURFACE)),
            self.notes,
            padding=16,
            expand=True,
            spacing=6,
        )
        self.log = ft.Text("", size=12, font_family="Cascadia Mono", selectable=True)
        details = ft.Container(
            ft.ExpansionTile(
                title=ft.Text("Detalles técnicos", size=14),
                leading=ft.Icon(ft.Icons.TERMINAL),
                controls=[ft.Container(ft.Column([self.log], scroll=ft.ScrollMode.AUTO), height=170,
                                       padding=ft.Padding.symmetric(horizontal=16, vertical=8))],
                shape=ft.RoundedRectangleBorder(radius=18),
                collapsed_shape=ft.RoundedRectangleBorder(radius=18),
                bgcolor=ft.Colors.SURFACE_CONTAINER,
                collapsed_bgcolor=ft.Colors.SURFACE_CONTAINER,
            ),
        )
        right = ft.Container(
            ft.Column([pictures, parts, details], spacing=12, expand=True),
            expand=True,
            padding=ft.Padding.only(left=6, top=10, right=24, bottom=20),
        )
        p.add(ft.Column([header, ft.Row([left, right], expand=True, vertical_alignment=ft.CrossAxisAlignment.STRETCH)],
                        expand=True, spacing=0))

    @staticmethod
    def _caption(icon, text: str) -> ft.Control:
        return ft.Row([ft.Icon(icon, size=16, color=MUTED), ft.Text(text, size=12, weight=ft.FontWeight.W_600, color=MUTED)], spacing=6)

    @staticmethod
    def _cell(control: ft.Control, flex: int, numeric: bool) -> ft.Container:
        return ft.Container(control, expand=flex, alignment=ft.Alignment.CENTER_RIGHT if numeric else ft.Alignment.CENTER_LEFT)

    @staticmethod
    def _placeholder(text: str = "Aparecerá al convertir") -> ft.Control:
        return ft.Column(
            [ft.Icon(ft.Icons.IMAGE_OUTLINED, size=40, color=ft.Colors.OUTLINE),
             ft.Text(text, size=12, color=ft.Colors.OUTLINE)],
            horizontal_alignment=ft.CrossAxisAlignment.CENTER,
            alignment=ft.MainAxisAlignment.CENTER,
            tight=True,
        )

    # ------------------------------------------------------------------ parts list
    def _add_row(self, stem: str, count: int = 1) -> None:
        name = ft.Row([ft.Text(stem, weight=ft.FontWeight.W_600, size=13, max_lines=1, overflow=ft.TextOverflow.ELLIPSIS)]
                      + ([count_badge(count)] if count > 1 else []), spacing=8, tight=True)
        cells = [self._cell(name, COLUMNS[0][1], False), self._cell(chip("waiting"), COLUMNS[1][1], False)]
        cells += [self._cell(ft.Text("", size=13), f, n) for _t, f, n in COLUMNS[2:]]
        box = ft.Container(
            ft.Row(cells, spacing=8),
            padding=ft.Padding.symmetric(horizontal=12, vertical=9),
            border_radius=12,
            on_click=lambda e, s=stem: self._select(s),
            ink=True,
        )
        self.rows[stem] = PartRow(box, cells)
        self.states[stem] = "waiting"
        self.list.controls.append(box)

    def _set_state(self, stem: str, state: str) -> None:
        self.states[stem] = state
        self.rows[stem].cells[1].content = chip(state)

    def _fill_row(self, part: PartResult) -> None:
        if part.stem not in self.rows:
            return
        state = "error" if not part.ok else ("warn" if part.matches_source is False else "ok")
        self._set_state(part.stem, state)
        c = self.rows[part.stem].cells
        c[2].content = ft.Text(f"{part.volume_mm3 / 1000:.2f} cm³" if part.volume_mm3 is not None else "—", size=13)
        c[3].content = ft.Text(f"{part.source_volume_mm3 / 1000:.2f} cm³" if part.source_volume_mm3 is not None else "—",
                               size=13, color=MUTED)
        c[4].content = ft.Text(part.repair or "—", size=12, color=MUTED, max_lines=1, overflow=ft.TextOverflow.ELLIPSIS)
        c[5].content = ft.Text(f"{part.seconds:.1f} s", size=12, color=MUTED)

    def _select(self, stem: str) -> None:
        self.selected = stem
        for s, row in self.rows.items():
            row.box.bgcolor = ft.Colors.with_opacity(0.10, ft.Colors.PRIMARY) if s == stem else None
        part = next((p for p in (self.report.parts if self.report else []) if p.stem == stem), None)
        if part is None:
            self.notes.value = f"{stem}: todavía no se ha convertido."
        else:
            self.notes.value = f"{part.stem}: " + ("; ".join(part.warnings) if part.warnings else "sin avisos.")
        self.page.update()

    # ------------------------------------------------------------------ choices
    async def pick_source(self, _e=None) -> None:
        start = str(Path(self.source).parent) if self.source else None
        files = await self.picker.pick_files(dialog_title="Ensamblaje o pieza de SolidWorks", initial_directory=start,
                                             file_type=ft.FilePickerFileType.CUSTOM,
                                             allowed_extensions=["sldasm", "SLDASM", "sldprt", "SLDPRT"])
        if files and files[0].path:
            self.source = files[0].path
            self._remember()
            self._show_source()

    async def pick_out(self, _e=None) -> None:
        path = await self.picker.get_directory_path(dialog_title="Carpeta de salida", initial_directory=self.out)
        if path:
            self.out = path
            self.out_text.value = path
            self.out_text.tooltip = path
            self._remember()
            self.page.update()

    def _show_source(self) -> None:
        self.list.controls.clear()
        self.rows.clear()
        self.states.clear()
        self.summary.value = ""
        src = Path(self.source) if self.source else None
        if src is None or not src.is_file():
            self.page.update()
            return
        self.src_name.value = src.stem
        png = preview_png(src)
        self.pic_src.content = (ft.Image(src=png, fit=ft.BoxFit.CONTAIN, border_radius=12) if png
                                else self._placeholder("Este fichero no guarda vista previa"))
        self.pic_out.content = self._placeholder()
        self.src_folder.value = f"Carpeta: {src.parent.name}"
        self.src_folder.tooltip = str(src.parent)
        if src.suffix.lower() == ".sldprt":
            self.stems = [src.stem]
            self.src_info.value = "Pieza suelta"
            self._add_row(src.stem)
        else:
            try:
                asm = read_assembly(src)
            except Exception as e:
                self.src_info.value = f"No se puede leer: {e}"
                self.page.update()
                return
            self.stems = list(dict.fromkeys(i.stem for i in asm.instances))
            self.src_info.value = f"Ensamblaje · {len(self.stems)} piezas · {len(asm.instances)} instancias"
            for stem in self.stems:
                self._add_row(stem, sum(1 for i in asm.instances if i.stem == stem))
        self.status.value = "Listo para convertir."
        self.page.update()

    def _remember(self, _e=None) -> None:
        theme = {ft.ThemeMode.LIGHT: "light", ft.ThemeMode.DARK: "dark"}.get(self.page.theme_mode, "system")
        self.settings.update(source=self.source, out=self.out, up_y=bool(self.up_switch.value), theme=theme)
        prefs.save(self.settings)

    def toggle_theme(self, _e=None) -> None:
        if self.page.theme_mode == ft.ThemeMode.SYSTEM:
            dark = (self.page.platform_brightness or ft.Brightness.LIGHT) != ft.Brightness.DARK
        else:
            dark = self.page.theme_mode != ft.ThemeMode.DARK
        self.page.theme_mode = ft.ThemeMode.DARK if dark else ft.ThemeMode.LIGHT
        self.theme_btn.selected = dark
        self._remember()
        self.page.update()

    # ------------------------------------------------------------------ conversion
    def start(self, _e=None) -> None:
        src = Path(self.source) if self.source else None
        if src is None or src.suffix.lower() not in SUPPORTED or not src.is_file():
            self._toast("Elige primero un ensamblaje (.SLDASM) o una pieza (.SLDPRT) de SolidWorks.", ft.Colors.ERROR)
            return
        self.report = None
        self.running = True
        self.log.value = ""
        self.notes.value = "Selecciona una pieza para ver sus avisos."
        self._show_source()
        self.convert_btn.disabled = True
        self.open_btn.disabled = self.folder_btn.disabled = True
        self.progress.value = None  # indeterminate until the first step reports
        self.status.value = f"Convirtiendo {src.stem}…"
        self.page.update()
        self.page.run_thread(self._work, src, Path(self.out), bool(self.up_switch.value))

    def _work(self, src: Path, out: Path, up_y: bool) -> None:
        try:
            report = convert(src, out, up_y=up_y, log=self._log_line, progress=self._on_progress)
            self._finish(report)
        except Exception as e:  # shown to the user, never a crash of the window
            self._fail(str(e))

    def _log_line(self, line: str) -> None:
        self.log.value += line + "\n"
        self.page.update()

    def _on_progress(self, stage: str, done: int, total: int, part: PartResult | None) -> None:
        steps = total + 2
        if stage == "start" and self.stems:
            self._set_state(self.stems[0], "running")
        elif stage == "part" and part is not None:
            self._fill_row(part)
            waiting = [s for s in self.stems if self.states.get(s) == "waiting"]
            if waiting:
                self._set_state(waiting[0], "running")
            self.status.value = f"Pieza {done} de {total}: {part.stem}"
        elif stage == "assembly":
            self.status.value = "Montando el ensamblaje…"
            done = total + 1
        elif stage == "picture":
            self.status.value = "Dibujando el resultado…"
            done = total + 2
        self.progress.value = done / max(steps, 1)
        self.page.update()

    def _finish(self, report: Report) -> None:
        self.running = False
        self.report = report
        self.convert_btn.disabled = False
        self.progress.value = 1
        good = sum(1 for p in report.parts if p.ok and p.matches_source is not False)
        self.summary.value = f"{good} de {len(report.parts)} coinciden con SolidWorks"
        placed = f" · {report.placed} de {report.instances} instancias colocadas" if report.instances > 1 else ""
        self.status.value = f"Terminado en {report.seconds:.0f} s{placed}."
        if report.output:
            self.open_btn.disabled = self.folder_btn.disabled = False
            folder = Path(report.output).parent
            f = folder / PREVIEW_RESULT
            if f.is_file():
                self.pic_out.content = ft.Image(src=f.read_bytes(), fit=ft.BoxFit.CONTAIN, border_radius=12)
        for w in report.warnings:
            self.log.value += f"Aviso: {w}\n"
        if report.ok():
            self._toast("Conversión terminada: todo coincide con SolidWorks.", ft.Colors.GREEN)
        else:
            self._toast("Conversión terminada con avisos: revisa las piezas marcadas.", ft.Colors.AMBER)
        self.page.update()
        done_file = os.environ.get("CAD2SC_DONE_FILE")
        if done_file:  # for automated checks of the window
            Path(done_file).write_text(self.status.value or "", encoding="utf-8")

    def _fail(self, message: str) -> None:
        self.running = False
        self.convert_btn.disabled = False
        self.progress.value = 0
        self.status.value = "La conversión no ha terminado."
        self.log.value += f"Error: {message}\n"
        self._toast(message, ft.Colors.ERROR)
        self.page.update()
        done_file = os.environ.get("CAD2SC_DONE_FILE")
        if done_file:
            Path(done_file).write_text(f"ERROR: {message}", encoding="utf-8")

    def _toast(self, text: str, color) -> None:
        self.page.show_dialog(ft.SnackBar(content=ft.Row([ft.Icon(ft.Icons.INFO_OUTLINE, color=color), ft.Text(text)], spacing=10),
                                          behavior=ft.SnackBarBehavior.FLOATING, show_close_icon=True, width=560,
                                          shape=ft.RoundedRectangleBorder(radius=12)))

    def open_result(self, _e=None) -> None:
        if not self.report or not self.report.output:
            return
        exe = prefs.find_solidcraft()
        if exe:
            subprocess.Popen([str(exe), self.report.output])
        else:
            os.startfile(self.report.output)  # type: ignore[attr-defined]

    def open_folder(self, _e=None) -> None:
        if self.report and self.report.output:
            os.startfile(Path(self.report.output).parent)  # type: ignore[attr-defined]


def main(page: ft.Page) -> None:
    ConverterApp(page)


def run(view: ft.AppView | None = ft.AppView.FLET_APP, port: int = 0) -> None:
    ft.run(main, view=view, port=port)


if __name__ == "__main__":
    run()

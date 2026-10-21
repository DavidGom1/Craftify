"""Window for converting a SolidWorks assembly into SolidCraft designs.

Run with ``python -m cad2solidcraft.gui`` or double-click ``Convertir a SolidCraft.pyw``.
Uses the Sun Valley theme (sv-ttk, Windows 11 look) when it is installed and the plain ttk
look otherwise.
"""

from __future__ import annotations

import json
import os
import queue
import subprocess
import threading
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, messagebox, ttk

from .sw.assembly import read_assembly
from .sw.convert import PREVIEW_RESULT, PREVIEW_SOURCE, PartResult, Report, convert_assembly

try:  # optional: Windows 11 look
    import sv_ttk
except ImportError:  # pragma: no cover
    sv_ttk = None
try:  # optional: follow the system's light or dark mode
    import darkdetect
except ImportError:  # pragma: no cover
    darkdetect = None

TITLE = "Convertir a SolidCraft"
DEFAULT_OUT = Path.home() / "Documents" / "SolidCraft convertidos"
SETTINGS = Path(os.environ.get("APPDATA", str(Path.home()))) / "cad2solidcraft" / "settings.json"
FONT = "Segoe UI Variable Text"
FONT_DISPLAY = "Segoe UI Variable Display"
PREVIEW_W, PREVIEW_H = 420, 300


def _find_solidcraft() -> Path | None:
    for base in (os.environ.get("ProgramFiles"), os.environ.get("LOCALAPPDATA")):
        if base:
            for p in (Path(base) / "SolidCraft" / "solidcraft.exe", Path(base) / "Programs" / "SolidCraft" / "solidcraft.exe"):
                if p.is_file():
                    return p
    return None


def _load_settings() -> dict:
    try:
        return json.loads(SETTINGS.read_text(encoding="utf-8"))
    except Exception:
        return {}


def _save_settings(data: dict) -> None:
    try:
        SETTINGS.parent.mkdir(parents=True, exist_ok=True)
        SETTINGS.write_text(json.dumps(data, indent=1, ensure_ascii=False), encoding="utf-8")
    except Exception:
        pass


class Card(ttk.Frame):
    """A titled group with a step number."""

    def __init__(self, master, step: str, title: str) -> None:
        super().__init__(master, padding=(16, 12), style="Card.TFrame")
        head = ttk.Frame(self, style="Card.TFrame")
        head.pack(fill="x", pady=(0, 8))
        ttk.Label(head, text=step, style="Step.TLabel", width=2, anchor="center").pack(side="left")
        ttk.Label(head, text=title, style="CardTitle.TLabel").pack(side="left", padx=(8, 0))
        self.body = ttk.Frame(self, style="Card.TFrame")
        self.body.pack(fill="both", expand=True)


class App(tk.Tk):
    def __init__(self) -> None:
        super().__init__()
        self.title(TITLE)
        self.geometry("1220x800")
        self.minsize(1040, 680)
        self.settings = _load_settings()
        self.events: queue.Queue = queue.Queue()
        self.report: Report | None = None
        self.images: dict[str, tk.PhotoImage] = {}
        self.rows: dict[str, str] = {}
        self.running = False

        self.source = tk.StringVar(value=self.settings.get("source", ""))
        self.out = tk.StringVar(value=self.settings.get("out", str(DEFAULT_OUT)))
        self.up_y = tk.BooleanVar(value=self.settings.get("up_y", True))
        system_dark = bool(darkdetect and (darkdetect.theme() or "").lower() == "dark")
        self.dark = tk.BooleanVar(value=self.settings.get("dark", system_dark))
        self.details = tk.BooleanVar(value=False)

        self._apply_theme()
        self._build()
        self._show_source_info()
        self.protocol("WM_DELETE_WINDOW", self._close)
        self.after(100, self.pump)

    # ------------------------------------------------------------------ look
    def _apply_theme(self) -> None:
        if sv_ttk is not None:
            sv_ttk.set_theme("dark" if self.dark.get() else "light")
        style = ttk.Style(self)
        dark = self.dark.get()
        bg = style.lookup("TFrame", "background") or ("#1c1c1c" if dark else "#fafafa")
        card = "#2b2b2b" if dark else "#ffffff"
        dim = "#a0a0a0" if dark else "#606060"
        accent = "#4cc2ff" if dark else "#005fb8"
        self.colors = {"bg": bg, "card": card, "dim": dim, "accent": accent,
                       "ok": "#6ccb5f" if dark else "#0f7b0f", "warn": "#fce100" if dark else "#9d5d00",
                       "error": "#ff99a4" if dark else "#c42b1c"}
        style.configure("Card.TFrame", background=card)
        style.configure("Header.TFrame", background=bg)
        style.configure("Title.TLabel", font=(FONT_DISPLAY, 20, "bold"), background=bg)
        style.configure("Subtitle.TLabel", font=(FONT, 10), foreground=dim, background=bg)
        style.configure("CardTitle.TLabel", font=(FONT, 11, "bold"), background=card)
        style.configure("Step.TLabel", font=(FONT, 10, "bold"), foreground="#ffffff" if not dark else "#000000", background=accent)
        style.configure("Body.TLabel", font=(FONT, 10), background=card)
        style.configure("Name.TLabel", font=(FONT, 11, "bold"), background=card)
        style.configure("Dim.TLabel", font=(FONT, 9), foreground=dim, background=card)
        style.configure("Status.TLabel", font=(FONT, 10), background=card)
        style.configure("PreviewTitle.TLabel", font=(FONT, 9, "bold"), foreground=dim, background=card)
        style.configure("Treeview", rowheight=28, font=(FONT, 10))
        style.configure("Treeview.Heading", font=(FONT, 9, "bold"))
        self.configure(background=bg)
        if hasattr(self, "table"):
            self._tag_rows()
        if hasattr(self, "log"):
            self.log.configure(background=card, foreground=style.lookup("TLabel", "foreground") or ("#ffffff" if dark else "#000000"),
                               insertbackground=accent)

    def _tag_rows(self) -> None:
        self.table.tag_configure("ok", foreground=self.colors["ok"])
        self.table.tag_configure("warn", foreground=self.colors["warn"])
        self.table.tag_configure("error", foreground=self.colors["error"])
        self.table.tag_configure("waiting", foreground=self.colors["dim"])

    def toggle_theme(self) -> None:
        self._apply_theme()
        self._remember()

    # ------------------------------------------------------------------ layout
    def _build(self) -> None:
        outer = ttk.Frame(self, padding=(20, 16, 20, 16), style="Header.TFrame")
        outer.pack(fill="both", expand=True)

        header = ttk.Frame(outer, style="Header.TFrame")
        header.pack(fill="x", pady=(0, 14))
        titles = ttk.Frame(header, style="Header.TFrame")
        titles.pack(side="left")
        ttk.Label(titles, text=TITLE, style="Title.TLabel").pack(anchor="w")
        ttk.Label(titles, text="Ensamblajes de SolidWorks a diseños de SolidCraft, pieza a pieza y comprobados con el original",
                  style="Subtitle.TLabel").pack(anchor="w")
        ttk.Checkbutton(header, text="Modo oscuro", variable=self.dark, command=self.toggle_theme,
                        style="Switch.TCheckbutton" if sv_ttk else "TCheckbutton").pack(side="right")

        body = ttk.Frame(outer, style="Header.TFrame")
        body.pack(fill="both", expand=True)
        body.columnconfigure(1, weight=1)
        body.rowconfigure(0, weight=1)

        # left: the three steps and the action
        left = ttk.Frame(body, style="Header.TFrame", width=380)
        left.grid(row=0, column=0, sticky="nsw", padx=(0, 16))

        c1 = Card(left, "1", "Ensamblaje de SolidWorks")
        c1.pack(fill="x", pady=(0, 10))
        self.src_name = ttk.Label(c1.body, text="Ninguno elegido", style="Name.TLabel", wraplength=330)
        self.src_name.pack(anchor="w")
        self.src_path = ttk.Label(c1.body, text="Un fichero .SLDASM; sus piezas se buscan en su carpeta",
                                  style="Dim.TLabel", wraplength=330)
        self.src_path.pack(anchor="w", pady=(2, 8))
        ttk.Button(c1.body, text="Elegir ensamblaje…", command=self.pick_source).pack(anchor="w")

        c2 = Card(left, "2", "Carpeta de salida")
        c2.pack(fill="x", pady=(0, 10))
        self.out_label = ttk.Label(c2.body, textvariable=self.out, style="Body.TLabel", wraplength=330)
        self.out_label.pack(anchor="w", pady=(0, 8))
        ttk.Button(c2.body, text="Cambiar…", command=self.pick_out).pack(anchor="w")

        c3 = Card(left, "3", "Opciones")
        c3.pack(fill="x", pady=(0, 14))
        ttk.Checkbutton(c3.body, text="Poner las piezas de pie", variable=self.up_y, command=self._remember,
                        style="Switch.TCheckbutton" if sv_ttk else "TCheckbutton").pack(anchor="w")
        ttk.Label(c3.body, text="SolidWorks usa Y como vertical y SolidCraft usa Z.", style="Dim.TLabel",
                  wraplength=330).pack(anchor="w", pady=(4, 0))

        self.convert_btn = ttk.Button(left, text="Convertir", command=self.start,
                                      style="Accent.TButton" if sv_ttk else "TButton")
        self.convert_btn.pack(fill="x", ipady=6)
        self.progress = ttk.Progressbar(left, mode="determinate", maximum=100)
        self.progress.pack(fill="x", pady=(10, 4))
        self.status = ttk.Label(left, text="Elige un ensamblaje para empezar.", style="Subtitle.TLabel", wraplength=360)
        self.status.pack(anchor="w")

        actions = ttk.Frame(left, style="Header.TFrame")
        actions.pack(fill="x", pady=(12, 0))
        self.open_btn = ttk.Button(actions, text="Abrir en SolidCraft", command=self.open_result, state="disabled",
                                   style="Accent.TButton" if sv_ttk else "TButton")
        self.open_btn.pack(side="left")
        self.folder_btn = ttk.Button(actions, text="Abrir carpeta", command=self.open_folder, state="disabled")
        self.folder_btn.pack(side="left", padx=(8, 0))

        # right: pictures, the parts table and the details
        right = ttk.Frame(body, style="Header.TFrame")
        right.grid(row=0, column=1, sticky="nsew")
        right.columnconfigure(0, weight=1)
        right.rowconfigure(1, weight=1)

        pics = ttk.Frame(right, style="Header.TFrame")
        pics.grid(row=0, column=0, sticky="ew")
        pics.columnconfigure(0, weight=1)
        pics.columnconfigure(1, weight=1)
        self.pic_src = self._preview(pics, "SOLIDWORKS · VISTA PREVIA GUARDADA", 0)
        self.pic_out = self._preview(pics, "SOLIDCRAFT · RESULTADO", 1)

        table_card = ttk.Frame(right, padding=(12, 10), style="Card.TFrame")
        table_card.grid(row=1, column=0, sticky="nsew", pady=(12, 0))
        table_card.columnconfigure(0, weight=1)
        table_card.rowconfigure(1, weight=1)
        ttk.Label(table_card, text="Piezas", style="CardTitle.TLabel").grid(row=0, column=0, sticky="w", pady=(0, 6))
        cols = ("estado", "volumen", "solidworks", "reparacion", "tiempo")
        self.table = ttk.Treeview(table_card, columns=cols, show="tree headings", selectmode="browse", height=8)
        self.table.heading("#0", text="Pieza", anchor="w")
        self.table.column("#0", width=200, anchor="w")
        for c, text, w, anchor in (("estado", "Estado", 150, "w"), ("volumen", "Volumen", 100, "e"),
                                   ("solidworks", "En SolidWorks", 110, "e"), ("reparacion", "Corrección", 150, "w"),
                                   ("tiempo", "Tiempo", 70, "e")):
            self.table.heading(c, text=text, anchor=anchor)
            self.table.column(c, width=w, anchor=anchor, stretch=c in ("estado", "reparacion"))
        self.table.grid(row=1, column=0, sticky="nsew")
        sb = ttk.Scrollbar(table_card, orient="vertical", command=self.table.yview)
        sb.grid(row=1, column=1, sticky="ns")
        self.table.configure(yscrollcommand=sb.set)
        self.table.bind("<<TreeviewSelect>>", self._show_part_notes)
        self.notes = ttk.Label(table_card, text="", style="Dim.TLabel", wraplength=700, justify="left")
        self.notes.grid(row=2, column=0, columnspan=2, sticky="w", pady=(6, 0))
        self._tag_rows()

        det = ttk.Frame(right, style="Header.TFrame")
        det.grid(row=2, column=0, sticky="ew", pady=(10, 0))
        ttk.Checkbutton(det, text="Detalles técnicos", variable=self.details, command=self._toggle_details,
                        style="Switch.TCheckbutton" if sv_ttk else "TCheckbutton").pack(anchor="w")
        self.log_frame = ttk.Frame(right, style="Card.TFrame", padding=8)
        self.log = tk.Text(self.log_frame, height=8, wrap="word", state="disabled", font=("Cascadia Mono", 9),
                           relief="flat", borderwidth=0)
        self.log.pack(fill="both", expand=True)
        self._apply_theme()

    def _preview(self, master, title: str, col: int) -> ttk.Label:
        card = ttk.Frame(master, padding=(12, 10), style="Card.TFrame")
        card.grid(row=0, column=col, sticky="nsew", padx=(0, 6) if col == 0 else (6, 0))
        ttk.Label(card, text=title, style="PreviewTitle.TLabel").pack(anchor="w")
        pic = ttk.Label(card, anchor="center", style="Dim.TLabel", text="—")
        pic.pack(fill="both", expand=True, pady=(6, 0), ipady=PREVIEW_H // 2 - 10)
        return pic

    def _toggle_details(self) -> None:
        if self.details.get():
            self.log_frame.grid(row=3, column=0, sticky="nsew", pady=(6, 0))
        else:
            self.log_frame.grid_forget()

    # ------------------------------------------------------------------ choices
    def pick_source(self) -> None:
        start = Path(self.source.get()).parent if self.source.get() else Path.home()
        path = filedialog.askopenfilename(title="Ensamblaje de SolidWorks", initialdir=str(start),
                                          filetypes=[("Ensamblajes de SolidWorks", "*.sldasm *.SLDASM")])
        if path:
            self.source.set(path)
            self._remember()
            self._show_source_info()

    def pick_out(self) -> None:
        path = filedialog.askdirectory(title="Carpeta de salida", initialdir=self.out.get() or str(Path.home()))
        if path:
            self.out.set(path)
            self._remember()

    def _show_source_info(self) -> None:
        src = Path(self.source.get()) if self.source.get() else None
        self.table.delete(*self.table.get_children())
        self.rows.clear()
        if src is None or not src.is_file():
            self.src_name.configure(text="Ninguno elegido")
            return
        self.src_name.configure(text=src.stem)
        try:
            asm = read_assembly(src)
        except Exception as e:
            self.src_path.configure(text=f"{src.parent}\nNo se puede leer: {e}")
            return
        stems = list(dict.fromkeys(i.stem for i in asm.instances))
        self.src_path.configure(text=f"{src.parent}\n{len(stems)} piezas · {len(asm.instances)} instancias")
        for stem in stems:
            count = sum(1 for i in asm.instances if i.stem == stem)
            label = f"{stem}  ×{count}" if count > 1 else stem
            self.rows[stem] = self.table.insert("", "end", text=label, values=("En espera", "", "", "", ""), tags=("waiting",))
        self.status.configure(text="Listo para convertir.")

    def _remember(self) -> None:
        self.settings.update(source=self.source.get(), out=self.out.get(), up_y=self.up_y.get(), dark=self.dark.get())
        _save_settings(self.settings)

    # ------------------------------------------------------------------ conversion
    def start(self) -> None:
        src = Path(self.source.get().strip('" '))
        if src.suffix.lower() != ".sldasm" or not src.is_file():
            messagebox.showwarning(TITLE, "Elige un ensamblaje de SolidWorks (.SLDASM) que exista.")
            return
        out = Path(self.out.get().strip('" ') or DEFAULT_OUT)
        self.report = None
        self.running = True
        self._set_log("")
        self._show_source_info()
        self._picture(self.pic_src, None)
        self._picture(self.pic_out, None)
        for b in (self.open_btn, self.folder_btn, self.convert_btn):
            b.configure(state="disabled")
        self.progress.configure(value=0)
        self.status.configure(text=f"Convirtiendo {src.stem}…")
        threading.Thread(target=self._work, args=(src, out, self.up_y.get()), daemon=True).start()

    def _work(self, src: Path, out: Path, up_y: bool) -> None:
        try:
            report = convert_assembly(
                src, out, up_y=up_y,
                log=lambda m: self.events.put(("log", m)),
                progress=lambda stage, done, total, part: self.events.put(("progress", (stage, done, total, part))),
            )
            self.events.put(("done", report))
        except Exception as e:  # shown to the user, never a crash of the window
            self.events.put(("error", str(e)))

    def pump(self) -> None:
        try:
            while True:
                kind, value = self.events.get_nowait()
                if kind == "log":
                    self._append_log(value)
                elif kind == "progress":
                    self._on_progress(*value)
                elif kind == "done":
                    self._finish(value)
                elif kind == "error":
                    self._fail(value)
        except queue.Empty:
            pass
        self.after(80, self.pump)

    def _on_progress(self, stage: str, done: int, total: int, part: PartResult | None) -> None:
        steps = total + 2  # every part, the assembly, the picture
        if stage == "start" and self.rows:
            first = next(iter(self.rows.values()))
            self.table.item(first, values=("Convirtiendo…", "", "", "", ""), tags=())
        elif stage == "part" and part is not None:
            self._fill_row(part)
            nxt = [r for s, r in self.rows.items() if self.table.set(r, "estado") == "En espera"]
            if nxt:
                self.table.item(nxt[0], values=("Convirtiendo…", "", "", "", ""), tags=())
            self.status.configure(text=f"Pieza {done} de {total}: {part.stem}")
        elif stage == "assembly":
            self.status.configure(text="Montando el ensamblaje…")
            done = total + 1
        elif stage == "picture":
            self.status.configure(text="Dibujando el resultado…")
            done = total + 2
        self.progress.configure(value=100 * done / max(steps, 1))

    def _fill_row(self, part: PartResult) -> None:
        row = self.rows.get(part.stem)
        if row is None:
            row = self.rows[part.stem] = self.table.insert("", "end", text=part.stem)
        vol = f"{part.volume_mm3 / 1000:.2f} cm³" if part.volume_mm3 is not None else ""
        sw = f"{part.source_volume_mm3 / 1000:.2f} cm³" if part.source_volume_mm3 is not None else ""
        if not part.ok:
            state, tag = "✖  Error", "error"
        elif part.matches_source is False:
            state, tag = "⚠  No coincide", "warn"
        elif part.matches_source:
            state, tag = "✔  Coincide", "ok"
        else:
            state, tag = "✔  Convertida", "ok"
        self.table.item(row, values=(state, vol, sw, part.repair or "", f"{part.seconds:.1f} s"), tags=(tag,))

    def _show_part_notes(self, _event=None) -> None:
        if not self.report:
            return
        sel = self.table.selection()
        stem = next((s for s, r in self.rows.items() if sel and r == sel[0]), None)
        part = next((p for p in self.report.parts if p.stem == stem), None)
        if part is None:
            self.notes.configure(text="")
            return
        text = "; ".join(part.warnings) if part.warnings else "Sin avisos."
        self.notes.configure(text=f"{part.stem}: {text}")

    def _finish(self, report: Report) -> None:
        self.running = False
        self.report = report
        self.convert_btn.configure(state="normal")
        self.progress.configure(value=100)
        folder = Path(report.output).parent if report.output else None
        if report.output:
            self.open_btn.configure(state="normal")
            self.folder_btn.configure(state="normal")
        ok_parts = sum(1 for p in report.parts if p.ok and p.matches_source is not False)
        text = (f"Terminado en {report.seconds:.0f} s: {ok_parts} de {len(report.parts)} piezas coinciden con SolidWorks, "
                f"{report.placed} de {report.instances} instancias colocadas.")
        if not report.ok():
            text += " Selecciona una pieza marcada para ver el motivo."
        self.status.configure(text=text)
        for w in report.warnings:
            self._append_log(f"Aviso: {w}")
        if folder:
            self._picture(self.pic_src, folder / PREVIEW_SOURCE)
            self._picture(self.pic_out, folder / PREVIEW_RESULT)

    def _fail(self, message: str) -> None:
        self.running = False
        self.convert_btn.configure(state="normal")
        self.progress.configure(value=0)
        self.status.configure(text="La conversión no ha terminado.")
        self._append_log(f"Error: {message}")
        messagebox.showerror(TITLE, message)

    def open_result(self) -> None:
        if not self.report or not self.report.output:
            return
        exe = _find_solidcraft()
        if exe:
            subprocess.Popen([str(exe), self.report.output])
        else:
            os.startfile(self.report.output)  # type: ignore[attr-defined]

    def open_folder(self) -> None:
        if self.report and self.report.output:
            os.startfile(Path(self.report.output).parent)  # type: ignore[attr-defined]

    def _close(self) -> None:
        if self.running and not messagebox.askyesno(TITLE, "La conversión sigue en marcha. ¿Cerrar de todos modos?"):
            return
        self._remember()
        self.destroy()

    # ------------------------------------------------------------------ helpers
    def _set_log(self, text: str) -> None:
        self.log.configure(state="normal")
        self.log.delete("1.0", "end")
        self.log.insert("end", text)
        self.log.configure(state="disabled")

    def _append_log(self, line: str) -> None:
        self.log.configure(state="normal")
        self.log.insert("end", line + "\n")
        self.log.see("end")
        self.log.configure(state="disabled")

    def _picture(self, label: ttk.Label, path: Path | None) -> None:
        if path is None or not path.is_file():
            label.configure(image="", text="—")
            return
        try:
            img = tk.PhotoImage(file=str(path))
        except tk.TclError:
            label.configure(image="", text="(sin imagen)")
            return
        factor = max(1, -(-img.width() // PREVIEW_W), -(-img.height() // PREVIEW_H))
        if factor > 1:
            img = img.subsample(factor, factor)
        self.images[str(label)] = img
        label.configure(image=img, text="")


def main() -> None:
    App().mainloop()


if __name__ == "__main__":
    main()

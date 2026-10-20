"""Window for converting a SolidWorks assembly into SolidCraft designs.

Run with ``python -m cad2solidcraft.gui`` or double-click ``Convertir a SolidCraft.pyw``.
"""

from __future__ import annotations

import os
import queue
import subprocess
import threading
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, messagebox, ttk

from .sw.convert import PREVIEW_RESULT, PREVIEW_SOURCE, Report, convert_assembly

TITLE = "Convertir a SolidCraft"
DEFAULT_OUT = Path.home() / "Documents" / "SolidCraft convertidos"


def _find_solidcraft() -> Path | None:
    for base in (os.environ.get("ProgramFiles"), os.environ.get("LOCALAPPDATA")):
        if base:
            for p in (Path(base) / "SolidCraft" / "solidcraft.exe", Path(base) / "Programs" / "SolidCraft" / "solidcraft.exe"):
                if p.is_file():
                    return p
    return None


class App(tk.Tk):
    def __init__(self) -> None:
        super().__init__()
        self.title(TITLE)
        self.minsize(820, 640)
        self.events: queue.Queue = queue.Queue()
        self.report: Report | None = None
        self.images: list[tk.PhotoImage] = []

        self.source = tk.StringVar()
        self.out = tk.StringVar(value=str(DEFAULT_OUT))
        self.up_y = tk.BooleanVar(value=True)

        pad = {"padx": 8, "pady": 4}
        frm = ttk.Frame(self, padding=10)
        frm.pack(fill="both", expand=True)
        frm.columnconfigure(1, weight=1)

        ttk.Label(frm, text="Ensamblaje de SolidWorks (.SLDASM)").grid(row=0, column=0, sticky="w", **pad)
        ttk.Entry(frm, textvariable=self.source).grid(row=0, column=1, sticky="ew", **pad)
        ttk.Button(frm, text="Examinar…", command=self.pick_source).grid(row=0, column=2, **pad)

        ttk.Label(frm, text="Carpeta de salida").grid(row=1, column=0, sticky="w", **pad)
        ttk.Entry(frm, textvariable=self.out).grid(row=1, column=1, sticky="ew", **pad)
        ttk.Button(frm, text="Examinar…", command=self.pick_out).grid(row=1, column=2, **pad)

        ttk.Checkbutton(
            frm, text="El modelo tiene Y como vertical (lo normal en SolidWorks): girarlo para que quede de pie",
            variable=self.up_y,
        ).grid(row=2, column=0, columnspan=3, sticky="w", **pad)

        bar = ttk.Frame(frm)
        bar.grid(row=3, column=0, columnspan=3, sticky="ew", **pad)
        self.convert_btn = ttk.Button(bar, text="Convertir", command=self.start)
        self.convert_btn.pack(side="left")
        self.open_btn = ttk.Button(bar, text="Abrir en SolidCraft", command=self.open_result, state="disabled")
        self.open_btn.pack(side="left", padx=8)
        self.folder_btn = ttk.Button(bar, text="Abrir carpeta", command=self.open_folder, state="disabled")
        self.folder_btn.pack(side="left")
        self.progress = ttk.Progressbar(bar, mode="indeterminate", length=180)
        self.progress.pack(side="right")

        self.status = ttk.Label(frm, text="Elige un ensamblaje para empezar.")
        self.status.grid(row=4, column=0, columnspan=3, sticky="w", **pad)

        pics = ttk.Frame(frm)
        pics.grid(row=5, column=0, columnspan=3, sticky="ew", **pad)
        pics.columnconfigure(0, weight=1)
        pics.columnconfigure(1, weight=1)
        ttk.Label(pics, text="SolidWorks").grid(row=0, column=0)
        ttk.Label(pics, text="SolidCraft").grid(row=0, column=1)
        self.pic_src = ttk.Label(pics, relief="groove", anchor="center")
        self.pic_src.grid(row=1, column=0, sticky="nsew", padx=4)
        self.pic_out = ttk.Label(pics, relief="groove", anchor="center")
        self.pic_out.grid(row=1, column=1, sticky="nsew", padx=4)

        frm.rowconfigure(6, weight=1)
        self.log = tk.Text(frm, height=12, wrap="word", state="disabled", font=("Consolas", 9))
        self.log.grid(row=6, column=0, columnspan=3, sticky="nsew", **pad)
        scroll = ttk.Scrollbar(frm, command=self.log.yview)
        scroll.grid(row=6, column=3, sticky="ns")
        self.log.configure(yscrollcommand=scroll.set)

        self.after(100, self.pump)

    # ------------------------------------------------------------------ actions
    def pick_source(self) -> None:
        path = filedialog.askopenfilename(
            title="Ensamblaje de SolidWorks", filetypes=[("Ensamblajes de SolidWorks", "*.sldasm *.SLDASM")]
        )
        if path:
            self.source.set(path)

    def pick_out(self) -> None:
        path = filedialog.askdirectory(title="Carpeta de salida", initialdir=self.out.get() or str(Path.home()))
        if path:
            self.out.set(path)

    def start(self) -> None:
        src = Path(self.source.get().strip('" '))
        if src.suffix.lower() != ".sldasm" or not src.is_file():
            messagebox.showwarning(TITLE, "Elige un ensamblaje de SolidWorks (.SLDASM) que exista.")
            return
        out = Path(self.out.get().strip('" ') or DEFAULT_OUT)
        self.report = None
        self.set_log("")
        self.show_picture(self.pic_src, None)
        self.show_picture(self.pic_out, None)
        for b in (self.open_btn, self.folder_btn):
            b.configure(state="disabled")
        self.convert_btn.configure(state="disabled")
        self.status.configure(text=f"Convirtiendo {src.name}…")
        self.progress.configure(mode="indeterminate", value=0)
        self.progress.start(12)
        threading.Thread(target=self.work, args=(src, out, self.up_y.get()), daemon=True).start()

    def work(self, src: Path, out: Path, up_y: bool) -> None:
        try:
            report = convert_assembly(src, out, up_y=up_y, log=lambda m: self.events.put(("log", m)))
            self.events.put(("done", report))
        except Exception as e:  # shown to the user, never a crash of the window
            self.events.put(("error", str(e)))

    def pump(self) -> None:
        try:
            while True:
                kind, value = self.events.get_nowait()
                if kind == "log":
                    self.append_log(value)
                elif kind == "done":
                    self.finish(value)
                elif kind == "error":
                    self.fail(value)
        except queue.Empty:
            pass
        self.after(100, self.pump)

    def finish(self, report: Report) -> None:
        self.progress.stop()
        self.progress.configure(mode="determinate", value=100 if report.ok() else 0)
        self.convert_btn.configure(state="normal")
        self.report = report
        folder = Path(report.output).parent if report.output else None
        if report.output:
            self.open_btn.configure(state="normal")
            self.folder_btn.configure(state="normal")
        ok_parts = sum(p.ok for p in report.parts)
        text = (
            f"Listo: {ok_parts} de {len(report.parts)} piezas, {report.placed} de {report.instances} "
            f"instancias colocadas, en {report.seconds} s."
        )
        if not report.ok():
            text += " Hay avisos: revisa el registro."
        self.status.configure(text=text)
        for w in report.warnings:
            self.append_log(f"Aviso: {w}")
        if folder:
            self.show_picture(self.pic_src, folder / PREVIEW_SOURCE)
            self.show_picture(self.pic_out, folder / PREVIEW_RESULT)

    def fail(self, message: str) -> None:
        self.progress.stop()
        self.progress.configure(mode="determinate", value=0)
        self.convert_btn.configure(state="normal")
        self.status.configure(text="La conversión no ha terminado.")
        self.append_log(f"Error: {message}")
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

    # ------------------------------------------------------------------ helpers
    def set_log(self, text: str) -> None:
        self.log.configure(state="normal")
        self.log.delete("1.0", "end")
        self.log.insert("end", text)
        self.log.configure(state="disabled")

    def append_log(self, line: str) -> None:
        self.log.configure(state="normal")
        self.log.insert("end", line + "\n")
        self.log.see("end")
        self.log.configure(state="disabled")

    def show_picture(self, label: ttk.Label, path: Path | None) -> None:
        if path is None or not path.is_file():
            label.configure(image="", text="")
            return
        try:
            img = tk.PhotoImage(file=str(path))
        except tk.TclError:
            label.configure(image="", text="(sin imagen)")
            return
        factor = max(1, -(-img.width() // 380))  # fit about 380 px wide
        if factor > 1:
            img = img.subsample(factor, factor)
        self.images.append(img)
        label.configure(image=img, text="")


def main() -> None:
    App().mainloop()


if __name__ == "__main__":
    main()

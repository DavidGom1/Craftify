"""Entry point of the packaged app (Convertir a SolidCraft.exe)."""

import multiprocessing

from cad2solidcraft.app import run

if __name__ == "__main__":
    multiprocessing.freeze_support()
    run()

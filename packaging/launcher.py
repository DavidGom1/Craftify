"""Entry point of the packaged app (Craftify.exe)."""

import multiprocessing

from cad2solidcraft.app import run

if __name__ == "__main__":
    multiprocessing.freeze_support()
    run()

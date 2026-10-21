"""Double-click to open the converter window (pythonw runs it without a console).

Opens the Flet window when Flet is installed, otherwise the plain Tkinter one.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

try:
    from cad2solidcraft.app import run
except ImportError:  # Flet not installed
    from cad2solidcraft.gui import main as run

run()

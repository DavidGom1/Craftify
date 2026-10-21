"""Double-click to open the converter window (pythonw runs it without a console)."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from cad2solidcraft.app import run  # noqa: E402

run()

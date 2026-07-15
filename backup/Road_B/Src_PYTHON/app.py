"""PyInstaller entry point — launches the PyQt6 GUI.

Kept at the package root so `pyinstaller app.py` bundles the whole
``mac_find_b`` package into a macOS .app.
"""

import sys

from mac_find_b.gui import main

if __name__ == "__main__":
    sys.exit(main())

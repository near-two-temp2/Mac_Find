"""PyInstaller / launch entry point for the GUI app.

Kept at repo root of this implementation so PyInstaller has a simple, stable
script target: ``pyinstaller ... main.py``.
"""

from mac_find_a.app import main

if __name__ == "__main__":
    raise SystemExit(main())

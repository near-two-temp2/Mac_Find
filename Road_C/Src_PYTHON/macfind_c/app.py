"""Unified entry point.

``python -m macfind_c.app`` (and the bundled ``.app``) launches the PyQt6 GUI.
Passing a subcommand (``index`` / ``search`` / ``status``) routes to the
headless CLI instead — handy for CI smoke tests inside the same binary.
"""

from __future__ import annotations

import sys

_CLI_COMMANDS = {"index", "search", "status"}
# Flags that must be answered on the CLI without pulling in PyQt6 (so they work
# on a machine — or CI step — where only numpy is installed).
_CLI_FLAGS = {"--version", "-h", "--help"}


def main() -> int:
    argv = sys.argv[1:]
    if argv and (argv[0] in _CLI_COMMANDS or argv[0] in _CLI_FLAGS):
        from .cli import main as cli_main

        return cli_main(argv)

    # Default (no args): GUI. Import lazily so the CLI path has no hard PyQt6
    # dependency.
    from .gui import main as gui_main

    return gui_main()


if __name__ == "__main__":
    raise SystemExit(main())

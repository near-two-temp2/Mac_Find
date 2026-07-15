"""Headless CLI entry point — used for CI smoke tests and scripting.

Subcommands:
    index   Build the binary index from one or more roots.
    search  Query the index (falls back to searchfs when the index is absent).
    status  Print which backend is active.

The GUI (``macfind_c.gui``) shares the exact same :class:`~macfind_c.engine.
HybridEngine`, so a green CLI smoke test exercises the real search path.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from . import __version__
from .engine import HybridEngine, Source


def _cmd_index(args: argparse.Namespace) -> int:
    from . import index as index_mod

    roots = args.roots or [str(Path.home())]
    out = Path(args.out) if args.out else index_mod.DEFAULT_INDEX_PATH
    print(f"Scanning {roots} (max {args.max}) -> {out}", file=sys.stderr)
    written = index_mod.build(roots, out_path=out, max_entries=args.max)
    view = index_mod.load(written)
    print(f"Wrote {view.entry_count:,} entries to {written}")
    return 0


def _cmd_search(args: argparse.Namespace) -> int:
    engine = HybridEngine(index_path=Path(args.index) if args.index else None)
    outcome = engine.search(
        args.query,
        limit=args.limit,
        dirs_only=args.dirs_only,
        files_only=args.files_only,
    )
    print(
        f"[{outcome.source.value}] {len(outcome.results)} results "
        f"in {outcome.elapsed_ms:.1f} ms "
        f"({outcome.total_candidates} candidates)",
        file=sys.stderr,
    )
    for r in outcome.results:
        marker = "/" if r.is_dir else " "
        print(f"{r.score:>6} {marker} {r.path}")
    # A smoke test just needs the pipeline to run without crashing; zero results
    # (e.g. no index and no searchfs on a Linux runner) is still success.
    return 0


def _cmd_status(args: argparse.Namespace) -> int:
    engine = HybridEngine(index_path=Path(args.index) if args.index else None)
    print(engine.status_line())
    if not engine.has_index and engine.index_error:
        print(f"  reason: {engine.index_error}", file=sys.stderr)
    return 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="macfind-c",
        description="MacFind Road_C (hybrid: binary index + searchfs fallback).",
    )
    p.add_argument("--version", action="version", version=f"macfind-c {__version__}")
    sub = p.add_subparsers(dest="command")

    pi = sub.add_parser("index", help="build the binary index")
    pi.add_argument("roots", nargs="*", help="roots to scan (default: $HOME)")
    pi.add_argument("-o", "--out", help="output .idx path")
    pi.add_argument(
        "-m", "--max", type=int, default=200_000, help="max entries to index"
    )
    pi.set_defaults(func=_cmd_index)

    ps = sub.add_parser("search", help="query the index / searchfs")
    ps.add_argument("query")
    ps.add_argument("-i", "--index", help="path to .idx file")
    ps.add_argument("-l", "--limit", type=int, default=100)
    ps.add_argument("-d", "--dirs-only", action="store_true")
    ps.add_argument("-f", "--files-only", action="store_true")
    ps.set_defaults(func=_cmd_search)

    pst = sub.add_parser("status", help="print active backend")
    pst.add_argument("-i", "--index", help="path to .idx file")
    pst.set_defaults(func=_cmd_status)

    return p


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if not getattr(args, "func", None):
        parser.print_help()
        return 0
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())

"""CLI entry points for scripting and CI smoke tests.

    mac-find-b index [ROOT ...] [--out DIR] [--max N]
    mac-find-b search QUERY [--index DIR] [--limit N] [--files] [--dirs]

These are intentionally headless so CI can exercise the engine without a
display server.
"""

from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

from . import engine


def _cmd_index(args: argparse.Namespace) -> int:
    roots = args.roots or [str(Path.home())]
    out_dir = Path(args.out) if args.out else engine.default_index_dir()

    print(f"[index] roots={roots}")
    print(f"[index] out={out_dir}")
    if args.max:
        print(f"[index] max_entries={args.max}")

    start = time.time()

    def progress(n: int) -> None:
        sys.stdout.write(f"\r[index] scanned {n} entries")
        sys.stdout.flush()

    idx = engine.build_index(roots, max_entries=args.max, progress=progress)
    sys.stdout.write("\n")
    engine.save_index(idx, out_dir)
    elapsed = time.time() - start
    print(f"[index] built {idx.count} entries in {elapsed:.2f}s → {out_dir}")
    return 0


def _cmd_search(args: argparse.Namespace) -> int:
    index_dir = Path(args.index) if args.index else engine.default_index_dir()
    if not engine.index_exists(index_dir):
        print(f"[search] no index at {index_dir}; run `index` first", file=sys.stderr)
        return 2

    idx = engine.load_index(index_dir)
    start = time.time()
    results = engine.search(
        idx,
        args.query,
        limit=args.limit,
        files_only=args.files,
        dirs_only=args.dirs,
    )
    elapsed_ms = (time.time() - start) * 1000
    print(f"[search] '{args.query}' → {len(results)} hits in {elapsed_ms:.1f}ms")
    for r in results:
        kind = "d" if r.is_dir else "f"
        print(f"{r.score:6d} {kind} {r.path}")
    return 0


def _cmd_smoke(args: argparse.Namespace) -> int:
    """Self-contained smoke test: build an index of this package, search it."""
    root = str(Path(__file__).resolve().parent.parent)
    print(f"[smoke] building index of {root}")
    idx = engine.build_index([root], max_entries=args.max)
    print(f"[smoke] indexed {idx.count} entries")
    if idx.count == 0:
        print("[smoke] FAIL: empty index", file=sys.stderr)
        return 1

    # Round-trip through disk to exercise save/load/mmap.
    tmp = Path(args.out) if args.out else Path(root) / ".smoke_index"
    engine.save_index(idx, tmp)
    reloaded = engine.load_index(tmp)
    if reloaded.count != idx.count:
        print("[smoke] FAIL: reload count mismatch", file=sys.stderr)
        return 1

    hits = engine.search(reloaded, "engine")
    print(f"[smoke] search 'engine' → {len(hits)} hits")
    for h in hits[:5]:
        print(f"        {h.score:5d} {h.path}")
    if not any("engine" in h.path for h in hits):
        print("[smoke] FAIL: expected to find engine.py", file=sys.stderr)
        return 1

    print("[smoke] OK")
    return 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="mac-find-b",
        description="Road_B (Python) — binary index + bitmask + fzf file search",
    )
    sub = p.add_subparsers(dest="command", required=True)

    pi = sub.add_parser("index", help="build a binary index")
    pi.add_argument("roots", nargs="*", help="roots to scan (default: $HOME)")
    pi.add_argument("--out", help="index output dir")
    pi.add_argument("--max", type=int, help="cap number of entries (for testing)")
    pi.set_defaults(func=_cmd_index)

    ps = sub.add_parser("search", help="search an existing index")
    ps.add_argument("query")
    ps.add_argument("--index", help="index dir")
    ps.add_argument("--limit", type=int, default=200)
    ps.add_argument("--files", action="store_true", help="files only")
    ps.add_argument("--dirs", action="store_true", help="directories only")
    ps.set_defaults(func=_cmd_search)

    psm = sub.add_parser("smoke", help="self-contained CI smoke test")
    psm.add_argument("--max", type=int, default=5000)
    psm.add_argument("--out", help="temp index dir")
    psm.set_defaults(func=_cmd_smoke)

    return p


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())

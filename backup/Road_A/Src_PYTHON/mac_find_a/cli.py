"""CLI entry point for mac_find_a.

Primarily exists as a CI smoke test: it can run on the macos-latest runner to
prove the ctypes searchfs binding loads and executes without a GUI/display.

Usage:
    python -m mac_find_a.cli [options] <search_term>
    python -m mac_find_a.cli --self-test        # binding sanity check, no term

Options:
    -d/--dirs-only  -f/--files-only  -s/--case-sensitive  -e/--exact-match
    -p/--skip-packages  -i/--skip-invisibles
    -m/--limit N    -v/--volume PATH
"""

from __future__ import annotations

import argparse
import sys

from . import searchfs_engine as engine


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="mac_find_a",
        description="Road_A live filename search via searchfs() (no index).",
    )
    p.add_argument("term", nargs="?", help="substring to match in filenames")
    p.add_argument("-d", "--dirs-only", action="store_true")
    p.add_argument("-f", "--files-only", action="store_true")
    p.add_argument("-s", "--case-sensitive", action="store_true")
    p.add_argument("-e", "--exact-match", action="store_true")
    p.add_argument("-p", "--skip-packages", action="store_true")
    p.add_argument("-i", "--skip-invisibles", action="store_true")
    p.add_argument("-m", "--limit", type=int, default=1000,
                   help="stop after N matches (0 = unlimited)")
    p.add_argument("-v", "--volume", default=None,
                   help="search a specific volume mount path")
    p.add_argument("--self-test", action="store_true",
                   help="verify the ctypes binding loads; exit without searching")
    return p


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)

    if not engine.searchfs_available():
        print(f"searchfs() unavailable (libSystem load error: "
              f"{engine.load_error()}). This tool only runs on macOS.",
              file=sys.stderr)
        # Self-test on a non-macOS box still "passes" the import; only the
        # syscall is unavailable. Report distinctly for CI clarity.
        return 3

    if args.self_test:
        # Prove the struct layout + symbol resolution are sane by probing the
        # root volume's capabilities. No filesystem walk required.
        ok = engine.volume_supports_searchfs("/")
        print(f"self-test: searchfs binding loaded OK; "
              f"'/' supports searchfs = {ok}")
        return 0

    if args.dirs_only and args.files_only:
        print("error: --dirs-only and --files-only are mutually exclusive",
              file=sys.stderr)
        return 2

    if not args.term:
        build_parser().print_usage(sys.stderr)
        return 2

    opts = engine.SearchOptions(
        dirs_only=args.dirs_only,
        files_only=args.files_only,
        case_sensitive=args.case_sensitive,
        exact_match=args.exact_match,
        skip_packages=args.skip_packages,
        skip_invisibles=args.skip_invisibles,
        limit=args.limit,
    )

    count = 0
    try:
        for path in engine.search(args.term, opts, volume=args.volume):
            print(path)
            count += 1
    except KeyboardInterrupt:
        pass
    print(f"\n{count} match(es).", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

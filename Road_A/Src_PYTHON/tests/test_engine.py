"""Lightweight tests for the searchfs engine.

Struct-layout and post-filter tests run on any platform (no syscall needed).
The live-search test is skipped when searchfs() is unavailable (non-macOS CI).
Run with:  python -m pytest tests/ -q   (or just execute this file directly).
"""

import ctypes
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from mac_find_a import searchfs_engine as e  # noqa: E402


def test_struct_sizes():
    # Byte-accurate against the macOS SDK headers.
    assert ctypes.sizeof(e.attrreference) == 8
    assert ctypes.sizeof(e.attrlist) == 24
    assert ctypes.sizeof(e.packed_result) == 20
    assert ctypes.sizeof(e.searchstate) == 556
    assert ctypes.sizeof(e.fsid) == 8
    assert ctypes.sizeof(e.fsobj_id) == 8


def test_options_flags():
    f = e._build_options_flags(e.SearchOptions())
    assert f & e.SRCHFS_START
    assert f & e.SRCHFS_MATCHFILES
    assert f & e.SRCHFS_MATCHDIRS
    assert f & e.SRCHFS_MATCHPARTIALNAMES

    f = e._build_options_flags(e.SearchOptions(dirs_only=True, exact_match=True))
    assert not (f & e.SRCHFS_MATCHFILES)
    assert f & e.SRCHFS_MATCHDIRS
    assert not (f & e.SRCHFS_MATCHPARTIALNAMES)


def test_post_filter():
    cs = e.SearchOptions(case_sensitive=True)
    assert e._post_filter("/a/Info.plist", "Info", cs) is True
    assert e._post_filter("/a/info.plist", "Info", cs) is False

    ex = e.SearchOptions(case_sensitive=True, exact_match=True)
    assert e._post_filter("/a/Info.plist", "Info.plist", ex) is True
    assert e._post_filter("/a/xInfo.plist", "Info.plist", ex) is False

    # Case-insensitive: kernel already matched, always keep.
    assert e._post_filter("/a/anything", "x", e.SearchOptions()) is True


def test_live_search():
    if not e.searchfs_available():
        print("SKIP live search: searchfs unavailable (non-macOS)")
        return
    if not e.volume_supports_searchfs("/"):
        print("SKIP live search: / does not support searchfs")
        return
    opts = e.SearchOptions(dirs_only=True, limit=3)
    results = list(e.search("Applications", opts))
    assert results, "expected at least one 'Applications' directory"
    assert all("Applications" in os.path.basename(p) or "Applications" in p
               for p in results)


if __name__ == "__main__":
    test_struct_sizes()
    test_options_flags()
    test_post_filter()
    test_live_search()
    print("all engine tests passed")

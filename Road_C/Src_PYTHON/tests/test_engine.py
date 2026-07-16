"""Unit tests for the Road_C hybrid engine, index, bitmask, and fuzzy scorer.

These run cross-platform (they never require macOS-only ``searchfs``), so they
pass on the CI runner and on the dev machine alike.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from macfind_c import bitmask, fuzzy, index
from macfind_c.engine import HybridEngine, Source


# --------------------------------------------------------------------------- #
# bitmask
# --------------------------------------------------------------------------- #
def test_bitmask_subset_matches():
    entry = int(bitmask.of_str("readme.md"))
    q = int(bitmask.of_str("md"))
    assert bitmask.matches(entry, q)


def test_bitmask_missing_class_rejected():
    entry = int(bitmask.of_str("readme"))
    q = int(bitmask.of_str("xyz"))  # x is absent from "readme"
    assert not bitmask.matches(entry, q)


def test_bitmask_case_insensitive():
    assert int(bitmask.of_str("ABC")) == int(bitmask.of_str("abc"))


# --------------------------------------------------------------------------- #
# fuzzy
# --------------------------------------------------------------------------- #
def test_fuzzy_subsequence_hits():
    assert fuzzy.score(b"abc", b"a-b-c") is not None
    assert fuzzy.score(b"abc", b"xxabcxx") is not None


def test_fuzzy_non_subsequence_misses():
    assert fuzzy.score(b"zzz", b"abc") is None


def test_fuzzy_contiguous_beats_scattered():
    contiguous = fuzzy.score(b"cat", b"cat.txt")
    scattered = fuzzy.score(b"cat", b"c_a_t.txt")
    assert contiguous is not None and scattered is not None
    assert contiguous[0] > scattered[0]


def test_fuzzy_empty_pattern():
    assert fuzzy.score(b"", b"anything") == (0, 0, 0)


# --------------------------------------------------------------------------- #
# rank_score — exact / substring must dominate scattered subsequence noise
# --------------------------------------------------------------------------- #
def _rank(query: str, path: str) -> int | None:
    p = path.encode()
    bn = p.rfind(b"/") + 1
    return fuzzy.rank_score(query.encode(), p, bn if bn >= 0 else 0)


def test_rank_exact_basename_wins():
    # Exact basename hit must beat a same-letters-but-scattered subsequence
    # (basename "t_e_m_p_-_t_e_s_t" contains temp_test only as a scattered run).
    exact = _rank("temp_test", "/users/oracle/temp_test")
    scattered = _rank("temp_test", "/x/txexmxpx_xtxexsxt")
    assert exact is not None and scattered is not None
    assert exact > scattered
    # A path missing a required letter isn't even a subsequence -> dropped.
    assert _rank("temp_test", "/users/oracle/vscode_pytest") is None


def test_rank_substring_beats_subsequence():
    # A contiguous "temp_test" in the basename must outrank a scattered match.
    sub = _rank("temp_test", "/x/my_temp_test_dir")
    scat = _rank("temp_test", "/x/t_e_m_p_t_e_s_t")
    assert sub is not None
    if scat is not None:  # scattered may or may not be a subsequence
        assert sub > scat


def test_rank_basename_beats_deep_path_match():
    # "config" in the basename beats "config" only appearing in a parent dir.
    bn = _rank("config", "/etc/app/config")
    deep = _rank("config", "/config/app/data.bin")
    assert bn is not None and deep is not None
    assert bn > deep


def test_rank_non_subsequence_is_none():
    assert _rank("zzzz", "/users/oracle/temp_test") is None


def test_engine_exact_dir_ranks_first(tmp_path: Path):
    # Build a tree where the exact "temp_test" dir competes with fzf noise.
    tree = tmp_path / "tree"
    (tree / "temp_test").mkdir(parents=True)
    (tree / "vscode_pytest").mkdir()
    (tree / "testing_tmp_helpers").mkdir()
    (tree / "temp_test" / "inner.txt").write_text("x")
    idx_path = tmp_path / "rank.idx"
    index.build([str(tree)], out_path=idx_path)

    engine = HybridEngine(index_path=idx_path)
    out = engine.search("temp_test")
    assert out.source == Source.INDEX
    assert out.results, "expected at least one hit"
    # The real basename-exact directory must be first.
    assert out.results[0].path.endswith("/temp_test")
    assert out.results[0].is_dir


# --------------------------------------------------------------------------- #
# volumes — local-path guard degrades safely off-macOS
# --------------------------------------------------------------------------- #
def test_volumes_local_scan_roots_nonempty():
    from macfind_c import volumes

    roots = volumes.local_scan_roots()
    assert isinstance(roots, list) and roots
    # Home must be in scope, and no known B2/cloud mount may leak in.
    assert any(str(Path.home()) == r or r.startswith(str(Path.home())) for r in roots)
    for r in roots:
        assert not r.startswith("/Volumes/Disk/h2-")


def test_volumes_denylist_rejected():
    from macfind_c import volumes

    assert volumes.is_local_path("/Volumes/Disk/h2-bu-01/anything") is False
    assert volumes.is_local_path("/Volumes/Disk/h2_bu_01_b2/x") is False


# --------------------------------------------------------------------------- #
# index build / load round-trip
# --------------------------------------------------------------------------- #
def _make_tree(root: Path) -> None:
    (root / "docs").mkdir()
    (root / "src").mkdir()
    (root / "docs" / "readme.md").write_text("hi")
    (root / "docs" / "guide.md").write_text("hi")
    (root / "src" / "main.py").write_text("hi")
    (root / "notes.txt").write_text("hi")


def test_index_roundtrip(tmp_path: Path):
    tree = tmp_path / "tree"
    tree.mkdir()
    _make_tree(tree)
    idx_path = tmp_path / "test.idx"

    index.build([str(tree)], out_path=idx_path)
    view = index.load(idx_path)

    assert view.entry_count >= 6  # 2 dirs + 4 files at least
    paths = [view.path_str(i) for i in range(view.entry_count)]
    assert any(p.endswith("readme.md") for p in paths)
    assert any(p.endswith("main.py") for p in paths)
    # At least one directory flag is set.
    assert any(view.is_dir(i) for i in range(view.entry_count))


def test_load_missing_raises(tmp_path: Path):
    with pytest.raises(index.IndexError):
        index.load(tmp_path / "nope.idx")


def test_load_corrupt_raises(tmp_path: Path):
    bad = tmp_path / "bad.idx"
    bad.write_bytes(b"NOTMAGIC" + b"\x00" * 100)
    with pytest.raises(index.IndexError):
        index.load(bad)


# --------------------------------------------------------------------------- #
# hybrid engine
# --------------------------------------------------------------------------- #
def test_engine_uses_index_when_present(tmp_path: Path):
    tree = tmp_path / "tree"
    tree.mkdir()
    _make_tree(tree)
    idx_path = tmp_path / "e.idx"
    index.build([str(tree)], out_path=idx_path)

    engine = HybridEngine(index_path=idx_path)
    assert engine.has_index

    out = engine.search("readme")
    assert out.source == Source.INDEX
    assert any("readme.md" in r.path for r in out.results)


def test_engine_dirs_only_filter(tmp_path: Path):
    tree = tmp_path / "tree"
    tree.mkdir()
    _make_tree(tree)
    idx_path = tmp_path / "e2.idx"
    index.build([str(tree)], out_path=idx_path)

    engine = HybridEngine(index_path=idx_path)
    out = engine.search("docs", dirs_only=True)
    assert out.source == Source.INDEX
    assert all(r.is_dir for r in out.results)


def test_engine_falls_back_when_index_missing(tmp_path: Path):
    # No index at this path -> engine must not crash; source is SEARCHFS or NONE
    # depending on platform (searchfs only exists on macOS).
    engine = HybridEngine(index_path=tmp_path / "absent.idx")
    assert not engine.has_index
    out = engine.search("python")
    assert out.source in (Source.SEARCHFS, Source.NONE)


def test_engine_empty_query_lists_entries(tmp_path: Path):
    tree = tmp_path / "tree"
    tree.mkdir()
    _make_tree(tree)
    idx_path = tmp_path / "e3.idx"
    index.build([str(tree)], out_path=idx_path)

    engine = HybridEngine(index_path=idx_path)
    out = engine.search("")
    assert out.source == Source.INDEX
    assert len(out.results) > 0

"""Hybrid search engine — the Road_C orchestrator.

Primary path: query the memory-mapped binary index with a numpy-vectorised
bitmask pre-filter (Phase 1) followed by fzf scoring on the survivors
(Phase 2). Fallback path: when the index is missing or corrupt, degrade to a
live ``searchfs()`` scan.

The engine is deliberately synchronous and dependency-light; the GUI drives it
from a background ``QThread`` (see :mod:`macfind_c.gui`).
"""

from __future__ import annotations

import time
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import List, Optional

import numpy as np

from . import bitmask, fuzzy, index, searchfs


class Source(Enum):
    """Which backend produced a result set."""

    INDEX = "index"
    SEARCHFS = "searchfs"
    NONE = "none"


@dataclass
class Result:
    """One search hit."""

    path: str
    score: int
    is_dir: bool

    @property
    def name(self) -> str:
        return self.path.rstrip("/").rsplit("/", 1)[-1] or self.path


@dataclass
class SearchOutcome:
    """A completed search: the hits plus how they were obtained."""

    results: List[Result]
    source: Source
    elapsed_ms: float
    total_candidates: int  # entries considered before scoring (index path only)


class HybridEngine:
    """Index-primary, searchfs-fallback filename search."""

    def __init__(self, index_path: Optional[Path] = None):
        self.index_path = Path(index_path) if index_path else index.DEFAULT_INDEX_PATH
        self._index: Optional[index.IndexView] = None
        self._index_error: Optional[str] = None
        self.load_index()

    # -- index lifecycle ---------------------------------------------------- #
    def load_index(self) -> bool:
        """(Re)load the index. Returns True on success, recording any error."""
        try:
            self._index = index.load(self.index_path)
            self._index_error = None
            return True
        except index.IndexError as e:
            self._index = None
            self._index_error = str(e)
            return False

    @property
    def has_index(self) -> bool:
        return self._index is not None

    @property
    def index_error(self) -> Optional[str]:
        return self._index_error

    @property
    def entry_count(self) -> int:
        return self._index.entry_count if self._index else 0

    def status_line(self) -> str:
        """Human-readable one-liner describing the active backend."""
        if self._index is not None:
            return f"Index: {self._index.entry_count:,} entries — {self.index_path}"
        base = "Index unavailable → searchfs() fallback"
        if searchfs.available():
            return base
        return base + " (searchfs unavailable on this platform)"

    # -- search ------------------------------------------------------------- #
    def search(
        self,
        query: str,
        limit: int = 500,
        dirs_only: bool = False,
        files_only: bool = False,
    ) -> SearchOutcome:
        """Run a query through the index, or fall back to searchfs()."""
        start = time.perf_counter()
        query = query.strip()

        if self._index is not None and self._index.entry_count > 0:
            results, candidates = self._search_index(
                query, limit, dirs_only, files_only
            )
            elapsed = (time.perf_counter() - start) * 1000.0
            return SearchOutcome(results, Source.INDEX, elapsed, candidates)

        # Fallback: live searchfs().
        if query and searchfs.available():
            raw = searchfs.search(
                query, dirs_only=dirs_only, files_only=files_only, limit=limit
            )
            results = [
                Result(path=p, score=0, is_dir=p.endswith("/")) for p in raw
            ]
            elapsed = (time.perf_counter() - start) * 1000.0
            return SearchOutcome(results, Source.SEARCHFS, elapsed, len(raw))

        elapsed = (time.perf_counter() - start) * 1000.0
        return SearchOutcome([], Source.NONE, elapsed, 0)

    def _search_index(
        self, query: str, limit: int, dirs_only: bool, files_only: bool
    ) -> tuple[List[Result], int]:
        """Two-phase index search: vectorised pre-filter, then fzf scoring."""
        idx = self._index
        assert idx is not None
        n = idx.entry_count

        # --- Phase 1: bitmask + kind pre-filter (fully vectorised) --------- #
        q_bytes = query.lower().encode("utf-8", "ignore")
        q_mask = np.uint64(int(bitmask.of_bytes(q_bytes)))

        if q_mask != 0:
            keep = (idx.masks & q_mask) == q_mask
        else:
            keep = np.ones(n, dtype=bool)

        if dirs_only:
            keep &= (idx.flags & index.FLAG_IS_DIR) != 0
        elif files_only:
            keep &= (idx.flags & index.FLAG_IS_DIR) == 0

        candidate_idx = np.nonzero(keep)[0]
        total_candidates = int(candidate_idx.size)

        # --- Phase 2: fzf scoring on survivors ----------------------------- #
        scored: List[Result] = []
        offsets = idx.offsets
        lengths = idx.lengths
        flags = idx.flags
        # Materialise the whole blob once as a contiguous bytes object: slicing
        # a Python bytes is far cheaper than slicing a np.memmap per entry, and
        # the survivors reference small windows into it.
        blob = idx.blob.tobytes()

        if not q_bytes:
            # Empty query: list survivors (bounded) without scoring.
            for i in candidate_idx[:limit]:
                i = int(i)
                s = int(offsets[i])
                e = s + int(lengths[i])
                path = blob[s:e].decode("utf-8", "replace")
                scored.append(Result(path, 0, bool(flags[i] & index.FLAG_IS_DIR)))
            return scored, total_candidates

        rank = fuzzy.rank_score  # local binding — hot loop
        # Keep the basename length alongside each hit so ties can be broken by
        # "more specific" (shorter) paths, matching the c-tauri standard.
        scored_rows: List[tuple[int, int, Result]] = []
        for i in candidate_idx:
            i = int(i)
            s = int(offsets[i])
            e = s + int(lengths[i])
            text = blob[s:e]
            # Basename offset: byte after the last '/'; 0 when the path has none.
            slash = text.rfind(b"/")
            bn_start = slash + 1 if slash >= 0 else 0
            hit = rank(q_bytes, text, bn_start)
            if hit is not None:
                path = text.decode("utf-8", "replace")
                scored_rows.append(
                    (
                        hit,
                        len(text),
                        Result(path, hit, bool(flags[i] & index.FLAG_IS_DIR)),
                    )
                )

        # Highest score first; ties broken by shorter total path (more specific).
        scored_rows.sort(key=lambda row: (-row[0], row[1]))
        scored = [row[2] for row in scored_rows[:limit]]
        return scored, total_candidates

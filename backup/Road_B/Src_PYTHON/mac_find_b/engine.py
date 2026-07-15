"""Binary index engine — Cling-inspired, implemented with numpy parallel arrays.

Design (see ../../../open-source-analysis.md §3):

  Index = a set of parallel arrays, one row per filesystem entry:
    masks[i]      uint64  letter bitmask of the whole lowercase path
    bn_masks[i]   uint64  letter bitmask of the basename only
    ext_ids[i]    uint32  small integer id of the file extension (0 = none)
    byte_off[i]   uint64  offset of the path bytes inside the packed blob
    byte_len[i]   uint16  length of the path bytes
    bn_start[i]   uint16  offset of the basename inside the path
    is_dir[i]     uint8   1 if directory
  Bulk data:
    all_bytes     one big lowercase-UTF-8 blob holding every path back-to-back

The arrays are written to disk with ``np.save`` into a directory and reloaded
with ``mmap_mode='r'`` so the OS pages them in lazily — the mmap-friendly layout
Cling uses, expressed in numpy.

Bitmask encoding (matches Cling):
    bits  0-25  letters a-z
    bits 26-35  digits 0-9
    bit   36    '.'
    bit   37    '-'
    bit   38    '_'

Search is two phase:
    Phase 1  numpy-vectorized prefilter:  (masks & q_mask) == q_mask
             plus optional extension-id equality.  O(n) over the whole column,
             but fully vectorized so it runs in C.
    Phase 2  fzf-style fuzzy scoring on the surviving candidates only, sorted
             by score descending.
"""

from __future__ import annotations

import json
import os
import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable, Iterator, List, Optional, Tuple

import numpy as np

INDEX_MAGIC = "MAC_FIND_B_IDX_V1"

# ---------------------------------------------------------------------------
# Bitmask
# ---------------------------------------------------------------------------

# Precompute a 256-entry lookup table: for each byte value, which bit it sets
# in the 64-bit mask (0 = contributes nothing).
_CHAR_BIT = np.zeros(256, dtype=np.uint64)
for _c in range(ord("a"), ord("z") + 1):
    _CHAR_BIT[_c] = np.uint64(1) << np.uint64(_c - ord("a"))
for _c in range(ord("0"), ord("9") + 1):
    _CHAR_BIT[_c] = np.uint64(1) << np.uint64(26 + (_c - ord("0")))
_CHAR_BIT[ord(".")] = np.uint64(1) << np.uint64(36)
_CHAR_BIT[ord("-")] = np.uint64(1) << np.uint64(37)
_CHAR_BIT[ord("_")] = np.uint64(1) << np.uint64(38)


def mask_of_bytes(data: bytes) -> int:
    """OR together the bit of every byte in ``data`` (already lowercased)."""
    m = np.uint64(0)
    for b in data:
        m |= _CHAR_BIT[b]
    return int(m)


# ---------------------------------------------------------------------------
# Extension ids
# ---------------------------------------------------------------------------


class ExtTable:
    """Maps extension strings <-> small integer ids.  id 0 means "no extension"."""

    def __init__(self, names: Optional[List[str]] = None) -> None:
        self._names: List[str] = names if names is not None else [""]
        self._ids = {name: i for i, name in enumerate(self._names)}

    def id_for(self, ext: str) -> int:
        i = self._ids.get(ext)
        if i is None:
            i = len(self._names)
            self._names.append(ext)
            self._ids[ext] = i
        return i

    def lookup(self, ext: str) -> Optional[int]:
        return self._ids.get(ext)

    @property
    def names(self) -> List[str]:
        return self._names


def _ext_of(basename: str) -> str:
    dot = basename.rfind(".")
    if dot <= 0 or dot == len(basename) - 1:
        return ""
    return basename[dot + 1 :].lower()


# ---------------------------------------------------------------------------
# Index data container
# ---------------------------------------------------------------------------


@dataclass
class Index:
    masks: np.ndarray       # uint64 [n]
    bn_masks: np.ndarray    # uint64 [n]
    ext_ids: np.ndarray     # uint32 [n]
    byte_off: np.ndarray    # uint64 [n]
    byte_len: np.ndarray    # uint16 [n]
    bn_start: np.ndarray    # uint16 [n]
    is_dir: np.ndarray      # uint8  [n]
    all_bytes: np.ndarray   # uint8  [total]
    ext_table: ExtTable

    @property
    def count(self) -> int:
        return int(self.masks.shape[0])

    def path_bytes(self, i: int) -> bytes:
        off = int(self.byte_off[i])
        ln = int(self.byte_len[i])
        return self.all_bytes[off : off + ln].tobytes()

    def path_str(self, i: int) -> str:
        return self.path_bytes(i).decode("utf-8", "replace")

    def basename_slice(self, i: int) -> Tuple[int, int]:
        start = int(self.bn_start[i])
        end = int(self.byte_len[i])
        return start, end


# ---------------------------------------------------------------------------
# Building
# ---------------------------------------------------------------------------

# Default roots to skip while walking — noisy, huge, or synthetic.
DEFAULT_SKIP_DIRS = {
    "/System/Volumes",
    "/private/var/vm",
    "/dev",
    "/.Spotlight-V100",
    "/.fseventsd",
    "/Volumes",  # avoid recursing into other mounted volumes / the index root
}

DEFAULT_SKIP_NAMES = {
    ".git",
    "node_modules",
    ".Trash",
    "Caches",
    ".build",
}


def _iter_paths(
    roots: Iterable[str],
    skip_names: set,
    max_entries: Optional[int],
    follow_symlinks: bool = False,
) -> Iterator[Tuple[str, bool]]:
    """Yield ``(path, is_dir)`` for every entry under ``roots``.

    Uses ``os.scandir`` (fast, uses the dirent d_type so it avoids extra
    ``stat`` calls — the moral equivalent of Cling's FTS_NOSTAT).
    """
    count = 0
    stack: List[str] = [os.path.abspath(r) for r in roots]
    while stack:
        current = stack.pop()
        try:
            with os.scandir(current) as it:
                for entry in it:
                    if entry.name in skip_names:
                        continue
                    try:
                        is_dir = entry.is_dir(follow_symlinks=follow_symlinks)
                    except OSError:
                        is_dir = False
                    yield entry.path, is_dir
                    count += 1
                    if max_entries is not None and count >= max_entries:
                        return
                    if is_dir:
                        stack.append(entry.path)
        except (PermissionError, FileNotFoundError, NotADirectoryError, OSError):
            continue


def build_index(
    roots: Iterable[str],
    *,
    max_entries: Optional[int] = None,
    skip_names: Optional[set] = None,
    progress: Optional[Callable[[int], None]] = None,
) -> Index:
    """Walk ``roots`` and build an in-memory :class:`Index`."""
    skip = set(DEFAULT_SKIP_NAMES if skip_names is None else skip_names)

    masks: List[int] = []
    bn_masks: List[int] = []
    ext_ids: List[int] = []
    byte_off: List[int] = []
    byte_len: List[int] = []
    bn_start: List[int] = []
    is_dir: List[int] = []
    blob = bytearray()
    ext_table = ExtTable()

    n = 0
    for path, isd in _iter_paths(roots, skip, max_entries):
        lower = path.lower().encode("utf-8", "replace")
        # A path longer than uint16 max is rare; clamp defensively.
        if len(lower) > 0xFFFF:
            lower = lower[:0xFFFF]
        off = len(blob)
        blob.extend(lower)

        sep = lower.rfind(b"/")
        bstart = sep + 1 if sep >= 0 else 0
        basename = lower[bstart:]

        masks.append(mask_of_bytes(lower))
        bn_masks.append(mask_of_bytes(basename))
        ext_ids.append(ext_table.id_for(_ext_of(basename.decode("utf-8", "replace"))))
        byte_off.append(off)
        byte_len.append(len(lower))
        bn_start.append(bstart)
        is_dir.append(1 if isd else 0)

        n += 1
        if progress is not None and (n & 0x3FFF) == 0:
            progress(n)

    if progress is not None:
        progress(n)

    return Index(
        masks=np.asarray(masks, dtype=np.uint64),
        bn_masks=np.asarray(bn_masks, dtype=np.uint64),
        ext_ids=np.asarray(ext_ids, dtype=np.uint32),
        byte_off=np.asarray(byte_off, dtype=np.uint64),
        byte_len=np.asarray(byte_len, dtype=np.uint16),
        bn_start=np.asarray(bn_start, dtype=np.uint16),
        is_dir=np.asarray(is_dir, dtype=np.uint8),
        all_bytes=np.frombuffer(bytes(blob), dtype=np.uint8),
        ext_table=ext_table,
    )


# ---------------------------------------------------------------------------
# Persistence (mmap-friendly)
# ---------------------------------------------------------------------------

_ARRAY_FILES = {
    "masks": "masks.npy",
    "bn_masks": "bn_masks.npy",
    "ext_ids": "ext_ids.npy",
    "byte_off": "byte_off.npy",
    "byte_len": "byte_len.npy",
    "bn_start": "bn_start.npy",
    "is_dir": "is_dir.npy",
    "all_bytes": "all_bytes.npy",
}


def default_index_dir() -> Path:
    base = Path.home() / "Library" / "Caches" / "com.macfind.roadb.python"
    return base / "index"


def save_index(index: Index, index_dir: os.PathLike | str) -> None:
    d = Path(index_dir)
    d.mkdir(parents=True, exist_ok=True)
    for attr, fname in _ARRAY_FILES.items():
        np.save(d / fname, getattr(index, attr))
    meta = {
        "magic": INDEX_MAGIC,
        "count": index.count,
        "ext_names": index.ext_table.names,
    }
    (d / "meta.json").write_text(json.dumps(meta), encoding="utf-8")


def load_index(index_dir: os.PathLike | str) -> Index:
    """Load an index, memory-mapping the big arrays (read-only)."""
    d = Path(index_dir)
    meta = json.loads((d / "meta.json").read_text(encoding="utf-8"))
    if meta.get("magic") != INDEX_MAGIC:
        raise ValueError(f"bad index magic in {d}")

    def load(name: str) -> np.ndarray:
        return np.load(d / _ARRAY_FILES[name], mmap_mode="r")

    return Index(
        masks=load("masks"),
        bn_masks=load("bn_masks"),
        ext_ids=load("ext_ids"),
        byte_off=load("byte_off"),
        byte_len=load("byte_len"),
        bn_start=load("bn_start"),
        is_dir=load("is_dir"),
        all_bytes=load("all_bytes"),
        ext_table=ExtTable(list(meta["ext_names"])),
    )


def index_exists(index_dir: os.PathLike | str) -> bool:
    d = Path(index_dir)
    if not (d / "meta.json").exists():
        return False
    return all((d / f).exists() for f in _ARRAY_FILES.values())


# ---------------------------------------------------------------------------
# Search
# ---------------------------------------------------------------------------


@dataclass
class SearchResult:
    path: str
    score: int
    is_dir: bool


# fzf-style scoring weights (mirrors Cling's table)
_SCORE_MATCH = 16
_SCORE_CONSECUTIVE = 4
_SCORE_FIRST_MULT = 2
_BONUS_BOUNDARY = 8
_GAP_START = -3
_GAP_EXTEND = -1

_BOUNDARY_BYTES = frozenset(b"/._- ")


def _fuzzy_score(pattern: bytes, text: bytes, bn_start: int) -> Optional[int]:
    """Greedy left-to-right fuzzy match with boundary/consecutive bonuses.

    Returns None if ``pattern`` is not a subsequence of ``text``; otherwise a
    score where higher is better.  ``bn_start`` is the offset of the basename so
    a match that lands inside the filename can be favoured.
    """
    if not pattern:
        return 0
    n = len(text)
    m = len(pattern)
    ti = 0
    pi = 0
    score = 0
    prev_match_idx = -2
    first = True
    while pi < m and ti < n:
        pc = pattern[pi]
        while ti < n and text[ti] != pc:
            ti += 1
        if ti >= n:
            return None  # remaining pattern char not found
        # matched pattern[pi] at text[ti]
        s = _SCORE_MATCH
        if ti == prev_match_idx + 1:
            s += _SCORE_CONSECUTIVE
        else:
            # gap between this match and the previous one
            gap = ti - prev_match_idx - 1
            if prev_match_idx >= 0 and gap > 0:
                s += _GAP_START + _GAP_EXTEND * min(gap - 1, 8)
        prev = text[ti - 1] if ti > 0 else ord("/")
        if prev in _BOUNDARY_BYTES or ti == bn_start:
            s += _BONUS_BOUNDARY
        if first:
            s *= _SCORE_FIRST_MULT
            first = False
        score += s
        prev_match_idx = ti
        ti += 1
        pi += 1
    if pi < m:
        return None
    # Reward matches that fall inside the basename.
    if prev_match_idx >= bn_start:
        score += _BONUS_BOUNDARY
    return score


def search(
    index: Index,
    query: str,
    *,
    limit: int = 200,
    files_only: bool = False,
    dirs_only: bool = False,
) -> List[SearchResult]:
    """Two-phase search over ``index``.

    Phase 1: numpy-vectorized bitmask prefilter + optional extension match.
    Phase 2: fzf scoring of the survivors, top ``limit`` by score.
    """
    q = query.strip().lower()
    if not q:
        return []

    q_bytes = q.encode("utf-8", "replace")

    # If the query looks like a bare extension filter (".py" / "*.py"), pull it
    # out so Phase 1 can use the extension column too.
    ext_filter_id: Optional[int] = None
    stripped = q
    if stripped.startswith("*."):
        stripped = stripped[2:]
    if stripped.startswith(".") and "/" not in stripped and stripped.count(".") == 1:
        ext = stripped[1:]
        if ext:
            ext_filter_id = index.ext_table.lookup(ext)
            if ext_filter_id is None:
                return []  # no file has that extension

    q_mask = np.uint64(mask_of_bytes(q_bytes))

    # ---- Phase 1: vectorized prefilter --------------------------------------
    # (masks & q_mask) == q_mask  →  the path contains at least the query's chars
    survive = (index.masks & q_mask) == q_mask

    if ext_filter_id is not None:
        survive &= index.ext_ids == np.uint32(ext_filter_id)

    if files_only:
        survive &= index.is_dir == 0
    elif dirs_only:
        survive &= index.is_dir == 1

    candidates = np.nonzero(survive)[0]
    if candidates.size == 0:
        return []

    # ---- Phase 2: fzf scoring ------------------------------------------------
    all_bytes = index.all_bytes
    byte_off = index.byte_off
    byte_len = index.byte_len
    bn_start_arr = index.bn_start
    is_dir_arr = index.is_dir

    # When filtering purely by extension the pattern is the extension text.
    score_pattern = q_bytes if ext_filter_id is None else stripped.lstrip(".").encode()

    scored: List[Tuple[int, int]] = []
    for idx in candidates:
        i = int(idx)
        off = int(byte_off[i])
        ln = int(byte_len[i])
        text = all_bytes[off : off + ln].tobytes()
        sc = _fuzzy_score(score_pattern, text, int(bn_start_arr[i]))
        if sc is not None:
            scored.append((sc, i))

    if not scored:
        return []

    # Highest score first; stable enough via secondary key on index.
    scored.sort(key=lambda t: (-t[0], t[1]))

    out: List[SearchResult] = []
    for sc, i in scored[:limit]:
        out.append(
            SearchResult(
                path=index.path_str(i),
                score=sc,
                is_dir=bool(is_dir_arr[i]),
            )
        )
    return out

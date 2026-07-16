"""Self-built binary file-name index (Cling-style parallel arrays).

The index is a single ``.idx`` file laid out so it can be loaded with
``np.memmap`` — no parsing loop, just typed views onto slices of the file. This
is the primary search structure for Road_C; :mod:`macfind_c.engine` falls back
to a live ``searchfs()`` scan when this file is missing or fails to validate.

On-disk layout (all little-endian)::

    Header (fixed 64 bytes):
        magic        8s   b"MACFIDX1"
        version      u32  format version (1)
        _pad         u32
        entry_count  u64  number of file entries
        bytes_count  u64  total length of the packed path blob
        off_masks    u64  file offset of masks[]      (u64 * entry_count)
        off_offsets  u64  file offset of byteOffsets[] (u64 * entry_count)
        off_lengths  u64  file offset of byteLengths[] (u32 * entry_count)
        off_flags    u64  file offset of flags[]       (u8  * entry_count)  bit0=isDir
        off_blob     u64  file offset of the lowercased-path blob

    Then, in order: masks, byteOffsets, byteLengths, flags, blob.

Each entry ``i`` describes one filesystem path:
    masks[i]       -> 64-bit character-class bitmask (see :mod:`macfind_c.bitmask`)
    byteOffsets[i] -> start of the path inside the blob
    byteLengths[i] -> path length in bytes
    flags[i]       -> bit 0 set means the path is a directory

Paths in the blob are lowercased UTF-8. The original-case path is recovered by
lowercasing being lossy only for display; for correctness we also keep an
original blob is unnecessary because macOS default search is case-insensitive —
we store the lowercased bytes and present them as-is. (Display fidelity is a
documented TODO; see README.)
"""

from __future__ import annotations

import os
import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Iterator, List, Optional

import numpy as np

from . import bitmask, volumes

MAGIC = b"MACFIDX1"
VERSION = 1
# Header is a fixed, padded block so the parallel arrays begin at a known,
# 8-byte-aligned offset (good for np.memmap). HEADER_STRUCT packs 72 bytes; we
# pad the on-disk header out to HEADER_SIZE, leaving room for future fields.
HEADER_STRUCT = struct.Struct("<8sIIQQQQQQQ")
HEADER_SIZE = 128
assert HEADER_STRUCT.size <= HEADER_SIZE

FLAG_IS_DIR = 0x01

# Default index location, mirroring Cling's cache convention.
DEFAULT_INDEX_PATH = (
    Path.home() / "Library" / "Caches" / "com.macfind.roadc.python" / "index.idx"
)

# Directories that are noise / permission traps during a scan.
_DEFAULT_EXCLUDES = (
    "/System",
    "/private/var/db",
    "/dev",
    "/.Spotlight-V100",
    "/.fseventsd",
    "/Library/Caches",
)


@dataclass
class IndexView:
    """A memory-mapped, read-only view onto an ``.idx`` file."""

    path: Path
    entry_count: int
    masks: np.ndarray  # uint64[entry_count]
    offsets: np.ndarray  # uint64[entry_count]
    lengths: np.ndarray  # uint32[entry_count]
    flags: np.ndarray  # uint8[entry_count]
    blob: np.ndarray  # uint8[bytes_count]

    def path_bytes(self, i: int) -> bytes:
        start = int(self.offsets[i])
        end = start + int(self.lengths[i])
        return self.blob[start:end].tobytes()

    def path_str(self, i: int) -> str:
        return self.path_bytes(i).decode("utf-8", "replace")

    def is_dir(self, i: int) -> bool:
        return bool(int(self.flags[i]) & FLAG_IS_DIR)


class IndexError(Exception):
    """Raised when an index file is missing, truncated, or corrupt."""


# --------------------------------------------------------------------------- #
# Building
# --------------------------------------------------------------------------- #
def _same_local_device(entry: os.DirEntry, base_dev: int, path: str) -> bool:
    """True if descending into ``entry`` is safe (local volume, on-list).

    Two guards, matching ``SEARCH_TEST_BASELINE.md``:
    1. Deny-list: a known cloud/FUSE mountpoint is refused *unconditionally*,
       even when it currently presents as plain APFS (a stale, unmounted
       placeholder shares its parent's ``st_dev`` and would otherwise pass).
    2. Device boundary: a change of ``st_dev`` means a nested mount — follow it
       only when that new volume is itself local.
    """
    if volumes.on_denylist(os.path.abspath(path)):
        return False
    try:
        dev = entry.stat(follow_symlinks=False).st_dev
    except OSError:
        return False
    if dev == base_dev:
        return True
    # Different device → a nested mount. Allow only if it's a local volume.
    return volumes.is_local_path(path)


def iter_paths(
    roots: Iterable[str],
    excludes: Iterable[str] = _DEFAULT_EXCLUDES,
    max_entries: Optional[int] = None,
    skip_network: bool = True,
) -> Iterator[tuple[str, bool]]:
    """Walk ``roots`` yielding ``(path, is_dir)`` while skipping ``excludes``.

    Uses ``os.scandir`` (which is backed by ``getattrlistbulk`` on macOS) for a
    fast, low-syscall traversal. Symlinks are not followed to avoid cycles.

    When ``skip_network`` is set (the default and the only safe mode on this
    machine), traversal will not descend into a directory that lives on a
    non-local volume: network/FUSE/cloud mounts are pruned by both
    :func:`volumes.is_local_path` (statfs / deny-list) and a per-root device
    boundary check (``st_dev``). This keeps the scan off the rclone→Backblaze
    B2 mounts, where recursion would burn API quota. See ``SEARCH_TEST_BASELINE``.
    """
    exclude_tuple = tuple(excludes)
    count = 0
    stack: List[str] = []
    # Record the originating device for each root so we never cross a mount
    # boundary into a network filesystem nested under a local root.
    root_dev: dict[str, int] = {}
    for r in (os.path.abspath(r) for r in roots):
        if skip_network and not volumes.is_local_path(r):
            continue
        try:
            root_dev[r] = os.lstat(r).st_dev
        except OSError:
            root_dev[r] = -1
        stack.append(r)

    # Map each queued directory to the st_dev of the root it descends from.
    dev_of: dict[str, int] = dict(root_dev)
    seen: set[str] = set()

    while stack:
        d = stack.pop()
        if d in seen:
            continue
        seen.add(d)
        base_dev = dev_of.get(d, -1)
        try:
            it = os.scandir(d)
        except (PermissionError, FileNotFoundError, NotADirectoryError, OSError):
            continue
        with it:
            for entry in it:
                p = entry.path
                if any(p.startswith(x) for x in exclude_tuple):
                    continue
                try:
                    is_dir = entry.is_dir(follow_symlinks=False)
                except OSError:
                    is_dir = False
                yield p, is_dir
                count += 1
                if max_entries is not None and count >= max_entries:
                    return
                if is_dir and not entry.is_symlink():
                    if skip_network and not _same_local_device(
                        entry, base_dev, p
                    ):
                        continue  # prune: mount boundary / non-local volume
                    dev_of[p] = base_dev
                    stack.append(p)


def build(
    roots: Iterable[str],
    out_path: os.PathLike | str = DEFAULT_INDEX_PATH,
    excludes: Iterable[str] = _DEFAULT_EXCLUDES,
    max_entries: Optional[int] = None,
    skip_network: bool = True,
) -> Path:
    """Scan ``roots`` and write a binary index to ``out_path``.

    Returns the written path. ``max_entries`` defaults to ``None`` — the whole
    local filesystem is indexed so nothing (e.g. ``~/temp_test``) is lost to an
    arbitrary cap; callers can still bound it for a quick CI smoke test.
    ``skip_network`` (on by default) keeps the scan off network/FUSE/cloud
    mounts; see :func:`iter_paths`.

    The whole entry set is materialised in memory before writing. Modern Macs
    have a few hundred thousand to a few million paths, whose parallel arrays +
    lowercased blob fit comfortably in RAM; a streaming builder remains a
    documented TODO for pathological volumes.
    """
    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    masks: List[int] = []
    lengths: List[int] = []
    flags: List[int] = []
    blob = bytearray()

    for p, is_dir in iter_paths(
        roots,
        excludes=excludes,
        max_entries=max_entries,
        skip_network=skip_network,
    ):
        lowered = p.lower().encode("utf-8", "ignore")
        masks.append(int(bitmask.of_bytes(lowered)))
        lengths.append(len(lowered))
        flags.append(FLAG_IS_DIR if is_dir else 0)
        blob.extend(lowered)

    n = len(masks)

    masks_arr = np.asarray(masks, dtype="<u8")
    lengths_arr = np.asarray(lengths, dtype="<u4")
    flags_arr = np.asarray(flags, dtype=np.uint8)
    # Reconstruct offsets from the cumulative lengths.
    offsets_arr = np.zeros(n, dtype="<u8")
    if n:
        np.cumsum(lengths_arr[:-1].astype("<u8"), out=offsets_arr[1:])
    blob_arr = np.frombuffer(bytes(blob), dtype=np.uint8)

    off_masks = HEADER_SIZE
    off_offsets = off_masks + masks_arr.nbytes
    off_lengths = off_offsets + offsets_arr.nbytes
    off_flags = off_lengths + lengths_arr.nbytes
    off_blob = off_flags + flags_arr.nbytes

    header = HEADER_STRUCT.pack(
        MAGIC,
        VERSION,
        0,
        n,
        len(blob),
        off_masks,
        off_offsets,
        off_lengths,
        off_flags,
        off_blob,
    )
    # Pad header to HEADER_SIZE.
    header = header + b"\x00" * (HEADER_SIZE - len(header))
    assert len(header) == HEADER_SIZE

    tmp = out_path.with_suffix(out_path.suffix + ".tmp")
    with open(tmp, "wb") as f:
        f.write(header)
        f.write(masks_arr.tobytes())
        f.write(offsets_arr.tobytes())
        f.write(lengths_arr.tobytes())
        f.write(flags_arr.tobytes())
        f.write(blob_arr.tobytes())
    os.replace(tmp, out_path)
    return out_path


# --------------------------------------------------------------------------- #
# Loading
# --------------------------------------------------------------------------- #
def load(path: os.PathLike | str = DEFAULT_INDEX_PATH) -> IndexView:
    """Memory-map an ``.idx`` file into an :class:`IndexView`.

    Raises :class:`IndexError` when the file is absent, truncated, or has a bad
    magic/version. The engine treats that as the trigger to fall back to
    ``searchfs()``.
    """
    path = Path(path)
    if not path.exists():
        raise IndexError(f"index not found: {path}")

    size = path.stat().st_size
    if size < HEADER_SIZE:
        raise IndexError(f"index too small ({size} bytes): {path}")

    with open(path, "rb") as f:
        raw_header = f.read(HEADER_SIZE)

    (
        magic,
        version,
        _pad,
        entry_count,
        bytes_count,
        off_masks,
        off_offsets,
        off_lengths,
        off_flags,
        off_blob,
    ) = HEADER_STRUCT.unpack(raw_header[: HEADER_STRUCT.size])

    if magic != MAGIC:
        raise IndexError(f"bad magic {magic!r} in {path}")
    if version != VERSION:
        raise IndexError(f"unsupported index version {version} in {path}")

    expected = off_blob + bytes_count
    if size < expected:
        raise IndexError(
            f"index truncated: file is {size} bytes, need {expected} ({path})"
        )

    masks = np.memmap(
        path, dtype="<u8", mode="r", offset=off_masks, shape=(entry_count,)
    )
    offsets = np.memmap(
        path, dtype="<u8", mode="r", offset=off_offsets, shape=(entry_count,)
    )
    lengths = np.memmap(
        path, dtype="<u4", mode="r", offset=off_lengths, shape=(entry_count,)
    )
    flags = np.memmap(
        path, dtype=np.uint8, mode="r", offset=off_flags, shape=(entry_count,)
    )
    blob = np.memmap(
        path, dtype=np.uint8, mode="r", offset=off_blob, shape=(bytes_count,)
    )

    return IndexView(
        path=path,
        entry_count=int(entry_count),
        masks=masks,
        offsets=offsets,
        lengths=lengths,
        flags=flags,
        blob=blob,
    )

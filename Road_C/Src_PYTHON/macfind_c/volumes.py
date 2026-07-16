"""Local-volume detection so the index never touches a network drive.

Indexing a network / FUSE mount is slow, can hang, and — critically on this
machine — the ``/Volumes/Disk/h2-*`` mounts are rclone → Backblaze B2, where a
recursive scan burns real API quota (money). So the scanner must stay on
**local** filesystems only.

The judgment mirrors ``SEARCH_TEST_BASELINE.md`` §"索引构建硬性要求":

1. Read each path's mount via ``statfs(2)`` and keep only local backing stores
   — ``f_fstypename`` in {apfs, hfs, …} and the ``MNT_LOCAL`` flag set. Any
   ``macfuse``/``nfs``/``smbfs``/``afpfs``/``webdav``/FileProvider mount is
   rejected.
2. During traversal, don't cross a device boundary (compare ``st_dev``) — that
   catches a network mount nested under a local root even if we forgot to name
   it.
3. Belt-and-suspenders: an explicit deny-list of the known B2/cloud mounts.

Everything degrades safely: if ``statfs`` is unavailable (non-macOS, or the
ctypes layer misbehaves) :func:`is_local_path` returns ``True`` so the caller
still works — the ``st_dev`` boundary check and the deny-list remain as guards.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import os
import platform
from pathlib import Path
from typing import Optional

_IS_MACOS = platform.system() == "Darwin"

# --- statfs(2) binding ----------------------------------------------------- #
# struct statfs (macOS, <sys/mount.h>): we only need f_flags and f_fstypename.
MFSTYPENAMELEN = 16
MNAMELEN = 1024
MNT_LOCAL = 0x00001000  # <sys/mount.h>: filesystem is stored locally

# Local filesystem type names we are willing to index.
_LOCAL_FSTYPES = frozenset(
    {"apfs", "hfs", "hfsplus", "exfat", "msdos", "ufs", "lifs"}
)

# Known network / FUSE / cloud mounts to reject outright, independent of statfs
# (see project CLAUDE.md and SEARCH_TEST_BASELINE.md). These are the documented
# rclone→Backblaze-B2 mountpoints in every naming variant we've seen (the
# project has used both `-` and `_` spellings). When a mount is active statfs
# already flags it macfuse; the deny-list additionally covers the stale-
# placeholder case where an unmounted path momentarily looks like plain APFS.
# Belt and suspenders — a stray recursive scan into B2 costs real money.
_DENY_PREFIXES = (
    "/Volumes/Disk/h2-bu-01",
    "/Volumes/Disk/h2_bu_01",       # also matches h2_bu_01_b2 via prefix check
    "/Volumes/Disk/h2-bu-01-b2",
    "/Volumes/Disk/h2_open_rsh",
    "/Volumes/Disk/h2-open-rsh",
    "/System/Volumes/Data/home",    # autofs
    "/net",
    "/home",
)


def on_denylist(ap: str) -> bool:
    """True when ``ap`` (an absolute path) is at/under a known cloud/FUSE mount."""
    for d in _DENY_PREFIXES:
        if ap == d or ap.startswith(d + "/") or ap.startswith(d + "_"):
            # The `d + "_"` arm folds `h2_bu_01_b2` under the `h2_bu_01` prefix
            # without over-matching an unrelated sibling directory.
            return True
    return False


class _Statfs(ctypes.Structure):
    # Field order per <sys/mount.h> (64-bit, non-deprecated `struct statfs`).
    _fields_ = [
        ("f_bsize", ctypes.c_uint32),
        ("f_iosize", ctypes.c_int32),
        ("f_blocks", ctypes.c_uint64),
        ("f_bfree", ctypes.c_uint64),
        ("f_bavail", ctypes.c_uint64),
        ("f_files", ctypes.c_uint64),
        ("f_ffree", ctypes.c_uint64),
        ("f_fsid", ctypes.c_int32 * 2),
        ("f_owner", ctypes.c_uint32),
        ("f_type", ctypes.c_uint32),
        ("f_flags", ctypes.c_uint32),
        ("f_fssubtype", ctypes.c_uint32),
        ("f_fstypename", ctypes.c_char * MFSTYPENAMELEN),
        ("f_mntonname", ctypes.c_char * MNAMELEN),
        ("f_mntfromname", ctypes.c_char * MNAMELEN),
        ("f_flags_ext", ctypes.c_uint32),
        ("f_reserved", ctypes.c_uint32 * 7),
    ]


def _load_statfs() -> Optional[ctypes.CDLL]:
    if not _IS_MACOS:
        return None
    try:
        name = ctypes.util.find_library("c") or "libSystem.dylib"
        libc = ctypes.CDLL(name, use_errno=True)
        # `statfs$INODE64` is the 64-bit variant CPython/libSystem uses; fall
        # back to plain `statfs` if the versioned symbol isn't present.
        for sym in ("statfs$INODE64", "statfs"):
            fn = getattr(libc, sym, None)
            if fn is not None:
                fn.restype = ctypes.c_int
                fn.argtypes = [ctypes.c_char_p, ctypes.POINTER(_Statfs)]
                libc._macfind_statfs = fn  # type: ignore[attr-defined]
                return libc
    except OSError:
        return None
    return None


_libc = _load_statfs()


def statfs_info(path: str) -> Optional[tuple[str, int]]:
    """Return ``(fstypename, f_flags)`` for ``path``, or ``None`` on failure."""
    if _libc is None:
        return None
    st = _Statfs()
    try:
        rc = _libc._macfind_statfs(  # type: ignore[attr-defined]
            os.fsencode(path), ctypes.byref(st)
        )
    except (OSError, ValueError):
        return None
    if rc != 0:
        return None
    fstype = st.f_fstypename.decode("ascii", "replace").lower()
    return fstype, int(st.f_flags)


def is_local_path(path: str) -> bool:
    """True when ``path`` lives on a local volume that is safe to index.

    Rejects anything on the known cloud/FUSE deny-list, then consults
    ``statfs``: a mount must be ``MNT_LOCAL`` *and* of a known local
    filesystem type. When ``statfs`` can't answer (non-macOS or symbol
    missing), returns ``True`` and leaves the ``st_dev`` boundary check +
    deny-list as the safety net.
    """
    ap = os.path.abspath(path)
    if on_denylist(ap):
        return False

    info = statfs_info(ap)
    if info is None:
        return True  # can't tell → don't block local scans on non-macOS/CI
    fstype, flags = info
    if not (flags & MNT_LOCAL):
        return False
    return fstype in _LOCAL_FSTYPES


def local_scan_roots() -> list[str]:
    """Default index roots: the user's home plus local ``/Volumes`` mounts.

    Skips every network/FUSE/cloud mount via :func:`is_local_path`, so a first
    build covers the local disk broadly without ever touching Backblaze B2.
    """
    roots: list[str] = []
    home = str(Path.home())
    if is_local_path(home):
        roots.append(home)

    apps = "/Applications"
    if os.path.isdir(apps) and is_local_path(apps):
        roots.append(apps)

    vol = "/Volumes"
    try:
        for name in os.listdir(vol):
            mount = os.path.join(vol, name)
            if not os.path.isdir(mount):
                continue
            if is_local_path(mount):
                roots.append(mount)
    except OSError:
        pass

    # De-dupe while preserving order; drop any root nested under an earlier one.
    seen: list[str] = []
    for r in roots:
        if not any(r == s or r.startswith(s + "/") for s in seen):
            seen.append(r)
    return seen or [home]

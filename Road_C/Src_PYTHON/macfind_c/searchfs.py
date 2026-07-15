"""Live ``searchfs()`` fallback engine via ctypes.

This is the Road_A degrade path used by Road_C when the binary index is
missing or corrupt. It reimplements the essential call sequence from the
reference C tool (``Open_Ref/searchfs/main.m``): pack an ``ATTR_CMN_NAME``
substring query into an ``fssearchblock``, ask the kernel for ``fsid``/``objid``
matches, then resolve each to a path with ``fsgetpath()``.

Only the machinery needed for a substring, files+dirs search is ported. All of
libSystem's ``searchfs`` / ``fsgetpath`` / ``getattrlist`` symbols are resolved
from ``libc`` via ctypes, so no compiled extension is required — which keeps the
PyInstaller bundle pure-Python.

If anything in the ctypes layer misbehaves (unexpected on non-macOS, or if a
future OS changes the ABI), :func:`available` returns ``False`` and callers get
an empty result instead of a crash.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import errno
import os
import platform
import struct
import time
from typing import Iterator, List, Optional

# --------------------------------------------------------------------------- #
# libc bindings
# --------------------------------------------------------------------------- #
_IS_MACOS = platform.system() == "Darwin"

PATH_MAX = 1024
MAX_MATCHES = 32
MAX_EBUSY_RETRIES = 5

# <sys/attr.h> constants (values verified against the SDK header).
ATTR_BIT_MAP_COUNT = 5
ATTR_CMN_NAME = 0x00000001
ATTR_CMN_FSID = 0x00000004
ATTR_CMN_OBJID = 0x00000020
# APFS returns a fake fsobj_id for ATTR_CMN_OBJID that fsgetpath() rejects with
# ENOTSUP. ATTR_CMN_FILEID returns the real 64-bit file/inode id, which is what
# fsgetpath() actually wants. We request FSID + FILEID for the return records.
ATTR_CMN_FILEID = 0x02000000

# searchfs() option flags — values must match <sys/attr.h> exactly (verified
# against the SDK header; note MATCHDIRS/MATCHFILES are 0x4/0x8, not 0x8/0x4).
SRCHFS_START = 0x00000001
SRCHFS_MATCHPARTIALNAMES = 0x00000002
SRCHFS_MATCHDIRS = 0x00000004
SRCHFS_MATCHFILES = 0x00000008
SRCHFS_SKIPLINKS = 0x00000010
SRCHFS_SKIPINVISIBLE = 0x00000020
SRCHFS_SKIPPACKAGES = 0x00000040
SRCHFS_NEGATEPARAMS = 0x80000000

DEFAULT_VOLUME = b"/"
DATA_VOLUME = b"/System/Volumes/Data"


class attrlist(ctypes.Structure):
    _fields_ = [
        ("bitmapcount", ctypes.c_ushort),
        ("reserved", ctypes.c_ushort),
        ("commonattr", ctypes.c_uint),
        ("volattr", ctypes.c_uint),
        ("dirattr", ctypes.c_uint),
        ("fileattr", ctypes.c_uint),
        ("forkattr", ctypes.c_uint),
    ]


class timeval(ctypes.Structure):
    _fields_ = [("tv_sec", ctypes.c_long), ("tv_usec", ctypes.c_int)]


class fssearchblock(ctypes.Structure):
    _fields_ = [
        ("returnattrs", ctypes.POINTER(attrlist)),
        ("returnbuffer", ctypes.c_void_p),
        ("returnbuffersize", ctypes.c_size_t),
        ("maxmatches", ctypes.c_ulong),
        ("timelimit", timeval),
        ("searchparams1", ctypes.c_void_p),
        ("sizeofsearchparams1", ctypes.c_size_t),
        ("searchparams2", ctypes.c_void_p),
        ("sizeofsearchparams2", ctypes.c_size_t),
        ("searchattrs", attrlist),
    ]


class attrreference(ctypes.Structure):
    _fields_ = [
        ("attr_dataoffset", ctypes.c_int),
        ("attr_length", ctypes.c_uint),
    ]


class packed_name_attr(ctypes.Structure):
    _fields_ = [
        ("size", ctypes.c_uint),
        ("ref", attrreference),
        ("name", ctypes.c_char * PATH_MAX),
    ]


class packed_attr_ref(ctypes.Structure):
    _fields_ = [
        ("size", ctypes.c_uint),
        ("ref", attrreference),
    ]


class fsid_t(ctypes.Structure):
    _fields_ = [("val", ctypes.c_int * 2)]


# The return buffer holds variable-size records, each laid out as:
#     u_int32_t size;   // length of this record, including the size field
#     fsid_t    fs_id;   // 2 x int32
#     u_int64_t file_id; // ATTR_CMN_FILEID
# We parse them with struct (walking by each record's own `size`) rather than a
# fixed ctypes stride, which is both simpler and robust to alignment padding.
_RECORD_HEADER = struct.Struct("<I ii Q")  # size, fsid[0], fsid[1], file_id


class searchstate(ctypes.Structure):
    # <sys/attr.h>: struct searchstate is 556 opaque bytes.
    _fields_ = [("reserved", ctypes.c_ubyte * 556)]


def _load_libc() -> Optional[ctypes.CDLL]:
    if not _IS_MACOS:
        return None
    try:
        name = ctypes.util.find_library("c") or "libSystem.dylib"
        return ctypes.CDLL(name, use_errno=True)
    except OSError:
        return None


_libc = _load_libc()

if _libc is not None:
    try:
        _libc.searchfs.restype = ctypes.c_int
        _libc.searchfs.argtypes = [
            ctypes.c_char_p,
            ctypes.POINTER(fssearchblock),
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.c_uint,
            ctypes.c_uint,
            ctypes.POINTER(searchstate),
        ]
        _libc.fsgetpath.restype = ctypes.c_ssize_t
        _libc.fsgetpath.argtypes = [
            ctypes.c_char_p,
            ctypes.c_size_t,
            ctypes.POINTER(fsid_t),
            ctypes.c_uint64,
        ]
    except AttributeError:
        _libc = None


def available() -> bool:
    """True when the ``searchfs``/``fsgetpath`` fallback can actually run."""
    return _IS_MACOS and _libc is not None


# --------------------------------------------------------------------------- #
# Search
# --------------------------------------------------------------------------- #
def _search_one_volume(
    volpath: bytes,
    term: str,
    dirs_only: bool,
    files_only: bool,
    limit: int,
    budget_s: float = 6.0,
) -> Iterator[str]:
    """Run the searchfs() loop against a single volume, yielding paths.

    ``budget_s`` bounds the total wall-clock time so a whole-volume scan can
    never hang the caller (the GUI thread or a CI step).
    """
    assert _libc is not None

    return_list = attrlist()
    return_list.bitmapcount = ATTR_BIT_MAP_COUNT
    # FSID + FILEID: the fake fsobj_id from ATTR_CMN_OBJID is rejected by
    # fsgetpath() on APFS (ENOTSUP); the real 64-bit file id resolves cleanly.
    return_list.commonattr = ATTR_CMN_FSID | ATTR_CMN_FILEID

    # Raw byte buffer: records are variable length; we walk them by each
    # record's own leading size field (see _RECORD_HEADER).
    buf_size = MAX_MATCHES * 64
    result_buffer = (ctypes.c_ubyte * buf_size)()

    blk = fssearchblock()
    blk.searchattrs.bitmapcount = ATTR_BIT_MAP_COUNT
    blk.searchattrs.commonattr = ATTR_CMN_NAME
    blk.returnattrs = ctypes.pointer(return_list)
    blk.returnbuffer = ctypes.cast(result_buffer, ctypes.c_void_p)
    blk.returnbuffersize = buf_size
    blk.maxmatches = MAX_MATCHES
    blk.timelimit = timeval(1, 0)

    name_bytes = term.encode("utf-8", "ignore")[: PATH_MAX - 1]
    info1 = packed_name_attr()
    info1.name = name_bytes
    info1.ref.attr_dataoffset = ctypes.sizeof(attrreference)
    info1.ref.attr_length = len(name_bytes) + 1
    info1.size = ctypes.sizeof(attrreference) + info1.ref.attr_length
    blk.searchparams1 = ctypes.cast(ctypes.pointer(info1), ctypes.c_void_p)
    blk.sizeofsearchparams1 = info1.size + ctypes.sizeof(ctypes.c_uint)

    info2 = packed_attr_ref()
    info2.ref.attr_dataoffset = ctypes.sizeof(attrreference)
    info2.ref.attr_length = 0
    info2.size = ctypes.sizeof(attrreference)
    blk.searchparams2 = ctypes.cast(ctypes.pointer(info2), ctypes.c_void_p)
    blk.sizeofsearchparams2 = ctypes.sizeof(info2)

    options = SRCHFS_START | SRCHFS_MATCHPARTIALNAMES
    if not dirs_only:
        options |= SRCHFS_MATCHFILES
    if not files_only:
        options |= SRCHFS_MATCHDIRS

    state = searchstate()
    matches = ctypes.c_ulong(0)
    ebusy = 0
    emitted = 0
    term_lower = term.lower()
    fsid = fsid_t()
    path_buf = ctypes.create_string_buffer(PATH_MAX)
    start = time.perf_counter()

    while True:
        matches.value = 0
        ctypes.set_errno(0)
        rc = _libc.searchfs(
            volpath,
            ctypes.byref(blk),
            ctypes.byref(matches),
            0,
            options,
            ctypes.byref(state),
        )
        err = ctypes.get_errno() if rc == -1 else 0

        if (rc == 0 or err == errno.EAGAIN) and matches.value > 0:
            data = bytes(result_buffer)
            off = 0
            for _ in range(matches.value):
                if off + _RECORD_HEADER.size > len(data):
                    break
                rec_size, fsid0, fsid1, file_id = _RECORD_HEADER.unpack_from(
                    data, off
                )
                off += rec_size if rec_size > 0 else _RECORD_HEADER.size

                fsid.val[0] = fsid0
                fsid.val[1] = fsid1
                n = _libc.fsgetpath(path_buf, PATH_MAX, ctypes.byref(fsid), file_id)
                if n > 0:
                    path = path_buf.value.decode("utf-8", "replace")
                    # Kernel already did case-insensitive substring; keep a
                    # cheap sanity filter on the basename.
                    if term_lower in os.path.basename(path).lower():
                        yield path
                        emitted += 1
                        if limit and emitted >= limit:
                            return

        if err == errno.EBUSY and ebusy < MAX_EBUSY_RETRIES:
            ebusy += 1
            options |= SRCHFS_START
            state = searchstate()
            matches.value = 0
            continue

        options &= ~SRCHFS_START
        if err != errno.EAGAIN:
            break
        if time.perf_counter() - start > budget_s:
            break


def search(
    term: str,
    dirs_only: bool = False,
    files_only: bool = False,
    limit: int = 1000,
    budget_s: float = 8.0,
) -> List[str]:
    """Live filename substring search via ``searchfs()``.

    Searches ``/`` and, on Catalina+, ``/System/Volumes/Data``. Returns at most
    ``limit`` paths within roughly ``budget_s`` seconds total. Returns ``[]``
    (never raises) when the fallback is unavailable so the engine can surface a
    clean "no results" state.
    """
    if not available() or not term:
        return []

    per_vol = budget_s / 2.0
    results: List[str] = []
    try:
        for p in _search_one_volume(
            DEFAULT_VOLUME, term, dirs_only, files_only, limit, budget_s=per_vol
        ):
            results.append(p)
            if len(results) >= limit:
                return results

        if os.path.isdir(DATA_VOLUME.decode()):
            remaining = limit - len(results)
            if remaining > 0:
                for p in _search_one_volume(
                    DATA_VOLUME,
                    term,
                    dirs_only,
                    files_only,
                    remaining,
                    budget_s=per_vol,
                ):
                    results.append(p)
                    if len(results) >= limit:
                        break
    except OSError:
        # A partial result set is still useful; return what we have.
        pass
    return results

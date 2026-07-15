"""Pure-ctypes binding to the macOS ``searchfs(2)`` system call.

Road_A engine: no index, every query performs a live catalog search over the
APFS/HFS+ B-Tree via the kernel ``searchfs()`` call, then reconstructs paths
with the private ``fsgetpath()`` SPI (fsid + objid -> path).

This is a direct port of ``Open_Ref/searchfs/main.m`` to ctypes.  All struct
byte layouts were verified against the macOS SDK headers (``sys/attr.h``,
``sys/vnode.h``, ``sys/fsgetpath.h``).

The module is import-safe on non-macOS platforms: ``libSystem`` simply won't
load and :func:`searchfs_available` returns ``False`` so the GUI / CLI can
degrade gracefully instead of crashing on import.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import os
from ctypes import (
    POINTER,
    c_char,
    c_char_p,
    c_int,
    c_int32,
    c_size_t,
    c_ubyte,
    c_uint32,
    c_ulong,
    c_void_p,
    sizeof,
)
from dataclasses import dataclass
from typing import Callable, Iterator, List, Optional

# ---------------------------------------------------------------------------
# Constants (from <sys/attr.h>)
# ---------------------------------------------------------------------------

ATTR_BIT_MAP_COUNT = 5
PATH_MAX = 1024

ATTR_CMN_NAME = 0x00000001
ATTR_CMN_FSID = 0x00000004
ATTR_CMN_OBJID = 0x00000020

ATTR_VOL_INFO = 0x80000000
ATTR_VOL_CAPABILITIES = 0x00020000

VOL_CAPABILITIES_INTERFACES = 1
VOL_CAP_INT_SEARCHFS = 0x00000001

SRCHFS_START = 0x00000001
SRCHFS_MATCHPARTIALNAMES = 0x00000002
SRCHFS_MATCHDIRS = 0x00000004
SRCHFS_MATCHFILES = 0x00000008
SRCHFS_SKIPLINKS = 0x00000010
SRCHFS_SKIPINVISIBLE = 0x00000020
SRCHFS_SKIPPACKAGES = 0x00000040
SRCHFS_NEGATEPARAMS = 0x80000000

MNT_NOWAIT = 2

# errno values we special-case
EBUSY = 16
EAGAIN = 35

MAX_MATCHES = 128          # results returned per searchfs() call
MAX_EBUSY_RETRIES = 5

DEFAULT_VOLUME = "/"
DATA_VOLUME = "/System/Volumes/Data"


# ---------------------------------------------------------------------------
# ctypes struct definitions (byte-accurate ports of the C structs)
# ---------------------------------------------------------------------------


class attrlist(ctypes.Structure):
    _fields_ = [
        ("bitmapcount", ctypes.c_ushort),
        ("reserved", ctypes.c_uint16),
        ("commonattr", c_uint32),
        ("volattr", c_uint32),
        ("dirattr", c_uint32),
        ("fileattr", c_uint32),
        ("forkattr", c_uint32),
    ]


class attrreference(ctypes.Structure):
    _fields_ = [
        ("attr_dataoffset", c_int32),
        ("attr_length", c_uint32),
    ]


class timeval(ctypes.Structure):
    _fields_ = [
        ("tv_sec", ctypes.c_long),
        ("tv_usec", c_int32),
    ]


class fssearchblock(ctypes.Structure):
    """struct fssearchblock — pointer-heavy, natural alignment."""

    _fields_ = [
        ("returnattrs", POINTER(attrlist)),
        ("returnbuffer", c_void_p),
        ("returnbuffersize", c_size_t),
        ("maxmatches", c_ulong),
        ("timelimit", timeval),
        ("searchparams1", c_void_p),
        ("sizeofsearchparams1", c_size_t),
        ("searchparams2", c_void_p),
        ("sizeofsearchparams2", c_size_t),
        ("searchattrs", attrlist),
    ]


class searchstate(ctypes.Structure):
    """struct searchstate — __attribute__((packed))."""

    _pack_ = 1
    _fields_ = [
        ("ss_union_flags", c_uint32),
        ("ss_union_layer", c_uint32),
        ("ss_fsstate", c_ubyte * 548),
    ]


class fsid(ctypes.Structure):
    _fields_ = [("val", c_int32 * 2)]


class fsobj_id(ctypes.Structure):
    _fields_ = [
        ("fid_objno", c_uint32),
        ("fid_generation", c_uint32),
    ]


class packed_name_attr(ctypes.Structure):
    """searchparams1 payload: size + attrreference + inline name bytes."""

    _fields_ = [
        ("size", c_uint32),
        ("ref", attrreference),
        ("name", c_char * PATH_MAX),
    ]


class packed_attr_ref(ctypes.Structure):
    """searchparams2 payload: size + attrreference (empty)."""

    _fields_ = [
        ("size", c_uint32),
        ("ref", attrreference),
    ]


class packed_result(ctypes.Structure):
    """One returnbuffer entry: size + fsid + fsobj_id."""

    _fields_ = [
        ("size", c_uint32),
        ("fs_id", fsid),
        ("obj_id", fsobj_id),
    ]


class vol_capabilities_set(ctypes.Structure):
    _fields_ = [("caps", c_uint32 * 4)]


class vol_capabilities_attr(ctypes.Structure):
    _fields_ = [
        ("capabilities", vol_capabilities_set),
        ("valid", vol_capabilities_set),
    ]


class vol_attr_buf(ctypes.Structure):
    _pack_ = 4
    _fields_ = [
        ("size", c_uint32),
        ("vol_capabilities", vol_capabilities_attr),
    ]


# ---------------------------------------------------------------------------
# libSystem binding (import-safe)
# ---------------------------------------------------------------------------

_libc = None
_load_error: Optional[str] = None

try:  # pragma: no cover - platform dependent
    _libc = ctypes.CDLL(ctypes.util.find_library("System") or "libSystem.dylib",
                        use_errno=True)

    _libc.searchfs.argtypes = [
        c_char_p,                    # path
        POINTER(fssearchblock),      # searchblock
        POINTER(c_ulong),            # nummatches (out)
        c_uint32,                    # scriptcode
        c_uint32,                    # options
        POINTER(searchstate),        # state
    ]
    _libc.searchfs.restype = c_int

    _libc.fsgetpath.argtypes = [c_char_p, c_size_t, POINTER(fsid), ctypes.c_uint64]
    _libc.fsgetpath.restype = ctypes.c_ssize_t

    _libc.getattrlist.argtypes = [c_char_p, c_void_p, c_void_p, c_size_t, c_uint32]
    _libc.getattrlist.restype = c_int
except OSError as exc:  # pragma: no cover
    _load_error = str(exc)
    _libc = None


def searchfs_available() -> bool:
    """True when the libSystem ``searchfs`` symbol is usable (i.e. on macOS)."""
    return _libc is not None


def load_error() -> Optional[str]:
    """Reason libSystem failed to load, if any (for diagnostics)."""
    return _load_error


# ---------------------------------------------------------------------------
# Options + result model
# ---------------------------------------------------------------------------


@dataclass
class SearchOptions:
    dirs_only: bool = False
    files_only: bool = False
    case_sensitive: bool = False
    exact_match: bool = False
    skip_packages: bool = False
    skip_invisibles: bool = False
    limit: int = 1000          # 0 == unlimited


def _build_options_flags(opts: SearchOptions) -> int:
    flags = SRCHFS_START
    if not opts.dirs_only:
        flags |= SRCHFS_MATCHFILES
    if not opts.files_only:
        flags |= SRCHFS_MATCHDIRS
    if not opts.exact_match:
        flags |= SRCHFS_MATCHPARTIALNAMES
    if opts.skip_packages:
        flags |= SRCHFS_SKIPPACKAGES
    if opts.skip_invisibles:
        flags |= SRCHFS_SKIPINVISIBLE
    return flags


def _post_filter(path: str, term: str, opts: SearchOptions) -> bool:
    """Return True if ``path`` should be kept.

    The kernel already did case-insensitive substring matching, so the only
    extra work needed is honoring ``case_sensitive`` (the kernel ignores case).
    ``exact_match`` is enforced by the kernel via dropping SRCHFS_MATCHPARTIALNAMES.
    """
    if opts.case_sensitive:
        base = os.path.basename(path)
        if opts.exact_match:
            return base == term
        return term in base
    return True


# ---------------------------------------------------------------------------
# Volume capability probe
# ---------------------------------------------------------------------------


def volume_supports_searchfs(path: str) -> bool:
    if _libc is None:
        return False
    al = attrlist()
    ctypes.memset(ctypes.byref(al), 0, sizeof(al))
    al.bitmapcount = ATTR_BIT_MAP_COUNT
    al.volattr = ATTR_VOL_INFO | ATTR_VOL_CAPABILITIES

    buf = vol_attr_buf()
    ctypes.memset(ctypes.byref(buf), 0, sizeof(buf))

    rc = _libc.getattrlist(path.encode("utf-8"), ctypes.byref(al),
                           ctypes.byref(buf), sizeof(buf), 0)
    if rc != 0:
        return False
    valid = buf.vol_capabilities.valid.caps[VOL_CAPABILITIES_INTERFACES]
    caps = buf.vol_capabilities.capabilities.caps[VOL_CAPABILITIES_INTERFACES]
    return bool(valid & VOL_CAP_INT_SEARCHFS) and bool(caps & VOL_CAP_INT_SEARCHFS)


def data_volume_available() -> bool:
    return os.path.exists(DATA_VOLUME) and volume_supports_searchfs(DATA_VOLUME)


# ---------------------------------------------------------------------------
# Core search
# ---------------------------------------------------------------------------


def _search_one_volume(
    volume: str,
    term: str,
    opts: SearchOptions,
    remaining: int,
    should_continue: Optional[Callable[[], bool]] = None,
) -> Iterator[str]:
    """Yield matching paths from a single volume.

    ``remaining`` is the residual result budget (0 == unlimited).  Loops over
    ``searchfs()`` while it returns EAGAIN (more results pending), retries on
    EBUSY (catalog changed mid-search), and stops early when ``should_continue``
    returns False (cooperative cancellation from the GUI worker thread).
    """
    assert _libc is not None
    term_bytes = term.encode("utf-8")

    # searchparams1: the name to match
    info1 = packed_name_attr()
    ctypes.memset(ctypes.byref(info1), 0, sizeof(info1))
    info1.name = term_bytes
    info1.ref.attr_dataoffset = sizeof(attrreference)
    info1.ref.attr_length = len(term_bytes) + 1
    info1.size = sizeof(attrreference) + info1.ref.attr_length

    # searchparams2: empty
    info2 = packed_attr_ref()
    info2.size = sizeof(attrreference)
    info2.ref.attr_dataoffset = sizeof(attrreference)
    info2.ref.attr_length = 0

    return_list = attrlist()
    ctypes.memset(ctypes.byref(return_list), 0, sizeof(return_list))
    return_list.bitmapcount = ATTR_BIT_MAP_COUNT
    return_list.commonattr = ATTR_CMN_FSID | ATTR_CMN_OBJID

    result_buffer = (packed_result * MAX_MATCHES)()

    blk = fssearchblock()
    ctypes.memset(ctypes.byref(blk), 0, sizeof(blk))
    blk.searchattrs.bitmapcount = ATTR_BIT_MAP_COUNT
    blk.searchattrs.commonattr = ATTR_CMN_NAME
    blk.returnattrs = ctypes.pointer(return_list)
    blk.returnbuffer = ctypes.cast(result_buffer, c_void_p)
    blk.returnbuffersize = sizeof(result_buffer)
    blk.searchparams1 = ctypes.cast(ctypes.pointer(info1), c_void_p)
    blk.sizeofsearchparams1 = info1.size + sizeof(c_uint32)
    blk.searchparams2 = ctypes.cast(ctypes.pointer(info2), c_void_p)
    blk.sizeofsearchparams2 = sizeof(info2)
    blk.maxmatches = MAX_MATCHES
    blk.timelimit.tv_sec = 1
    blk.timelimit.tv_usec = 0

    state = searchstate()
    nummatches = c_ulong(0)
    options = _build_options_flags(opts)
    ebusy_count = 0
    emitted = 0
    vol_bytes = volume.encode("utf-8")
    path_buf = ctypes.create_string_buffer(PATH_MAX)

    while True:
        if should_continue is not None and not should_continue():
            return

        nummatches.value = 0
        rc = _libc.searchfs(vol_bytes, ctypes.byref(blk), ctypes.byref(nummatches),
                            0, options, ctypes.byref(state))
        err = ctypes.get_errno() if rc == -1 else 0

        if (err == 0 or err == EAGAIN) and nummatches.value > 0:
            for i in range(nummatches.value):
                res = result_buffer[i]
                objid = (res.obj_id.fid_objno |
                         (res.obj_id.fid_generation << 32))
                size = _libc.fsgetpath(path_buf, PATH_MAX,
                                       ctypes.byref(res.fs_id), objid)
                if size > -1:
                    path = path_buf.raw[:size].decode("utf-8", "replace")
                    if _post_filter(path, term, opts):
                        yield path
                        emitted += 1
                        if remaining and emitted >= remaining:
                            return
                # size <= -1: object vanished between match and lookup; skip.

        if err == EBUSY and ebusy_count < MAX_EBUSY_RETRIES:
            ebusy_count += 1
            # Restart the whole search (SRCHFS_START still set).
            state = searchstate()
            continue

        if err != 0 and err != EAGAIN:
            # Unrecoverable error (EPERM without full-disk access, etc.).
            return

        # Clear SRCHFS_START for subsequent continuation calls.
        options &= ~SRCHFS_START
        if err != EAGAIN:
            break


def search(
    term: str,
    opts: Optional[SearchOptions] = None,
    volume: Optional[str] = None,
    should_continue: Optional[Callable[[], bool]] = None,
) -> Iterator[str]:
    """Live filename search via ``searchfs()``.

    Searches ``/`` and (on Catalina+) ``/System/Volumes/Data`` by default, or a
    single ``volume`` when specified.  Yields absolute paths lazily so the GUI
    can stream results.  ``should_continue`` is polled to allow cancellation.
    """
    if _libc is None:
        raise RuntimeError(
            "searchfs() unavailable: libSystem did not load (not macOS?)")
    if not term:
        return
    opts = opts or SearchOptions()

    total = 0
    volumes: List[str]
    if volume:
        volumes = [volume]
    else:
        volumes = [DEFAULT_VOLUME]
        if data_volume_available():
            volumes.append(DATA_VOLUME)

    for vol in volumes:
        if not volume_supports_searchfs(vol):
            continue
        remaining = 0 if not opts.limit else max(0, opts.limit - total)
        if opts.limit and remaining == 0:
            break
        for path in _search_one_volume(vol, term, opts, remaining, should_continue):
            yield path
            total += 1
            if opts.limit and total >= opts.limit:
                return

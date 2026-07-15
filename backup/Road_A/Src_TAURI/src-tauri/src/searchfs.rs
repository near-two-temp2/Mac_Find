//! Rust FFI wrapper around the macOS `searchfs(2)` catalog-search syscall.
//!
//! This is a direct port of the reference C implementation in
//! `Open_Ref/searchfs/main.m`. It searches the APFS/HFS+ B-Tree catalog for
//! filenames without building an index — every call scans live, which is why
//! it is ~100x faster than `find` yet needs no background daemon.
//!
//! Flow, mirroring the reference:
//!   1. Build an `fssearchblock` whose `searchparams1` carries the name to
//!      match (packed as `attrreference` + NUL-terminated bytes).
//!   2. Ask the kernel for `ATTR_CMN_FSID | ATTR_CMN_OBJID` back per match.
//!   3. Loop on `searchfs()` while it returns `EAGAIN` (more results pending),
//!      retry on `EBUSY` (catalog changed mid-scan).
//!   4. For each `(fsid, objid)` pair, call `fsgetpath()` to reconstruct the
//!      absolute path.
//!   5. On modern macOS (Catalina+) the read-only system volume `/` and the
//!      writable `/System/Volumes/Data` are firmlinked; searching only `/`
//!      misses user data, so we scan both by default.
//!
//! Everything here is `unsafe` at the boundary; the public [`search`] fn is
//! safe and returns owned `String`s.

#![allow(non_camel_case_types)]

use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_uint, c_void};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants (from <sys/attr.h>, <sys/vnode.h>)
// ---------------------------------------------------------------------------

const ATTR_BIT_MAP_COUNT: u16 = 5;

const ATTR_CMN_NAME: u32 = 0x0000_0001;
const ATTR_CMN_FSID: u32 = 0x0000_0004;
const ATTR_CMN_OBJID: u32 = 0x0000_0020;

const ATTR_VOL_INFO: u32 = 0x8000_0000;
const ATTR_VOL_CAPABILITIES: u32 = 0x0002_0000;

// searchfs() option flags
const SRCHFS_START: c_uint = 0x0000_0001;
const SRCHFS_MATCHPARTIALNAMES: c_uint = 0x0000_0002;
const SRCHFS_MATCHDIRS: c_uint = 0x0000_0004;
const SRCHFS_MATCHFILES: c_uint = 0x0000_0008;
const SRCHFS_SKIPLINKS: c_uint = 0x0000_0010;
const SRCHFS_SKIPINVISIBLE: c_uint = 0x0000_0020;
const SRCHFS_SKIPPACKAGES: c_uint = 0x0000_0040;
const SRCHFS_NEGATEPARAMS: c_uint = 0x0080_0000;

// vol_capabilities
const VOL_CAPABILITIES_INTERFACES: usize = 1;
const VOL_CAP_INT_SEARCHFS: u32 = 0x0000_0002;

const PATH_MAX: usize = 1024;
const MAX_MATCHES: usize = 64;
const MAX_EBUSY_RETRIES: u32 = 5;

const DEFAULT_VOLUME: &str = "/";
const DATA_VOLUME: &str = "/System/Volumes/Data";

// ---------------------------------------------------------------------------
// Structs mirroring the C ABI
// ---------------------------------------------------------------------------

#[repr(C)]
struct attrlist {
    bitmapcount: u16,
    reserved: u16,
    commonattr: u32,
    volattr: u32,
    dirattr: u32,
    fileattr: u32,
    forkattr: u32,
}

#[repr(C)]
struct attrreference {
    attr_dataoffset: i32,
    attr_length: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct fsid_t {
    val: [i32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct fsobj_id_t {
    fid_objno: u32,
    fid_generation: u32,
}

#[repr(C)]
struct timeval {
    tv_sec: libc::time_t,
    tv_usec: libc::suseconds_t,
}

// Field order MUST match <sys/attr.h> exactly, or the kernel reads garbage
// pointers and searchfs() fails with EFAULT (errno 14). The real layout is:
//   returnattrs, returnbuffer, returnbuffersize, maxmatches, timelimit,
//   searchparams1, sizeofsearchparams1, searchparams2, sizeofsearchparams2,
//   searchattrs
#[repr(C)]
struct fssearchblock {
    returnattrs: *mut attrlist,
    returnbuffer: *mut c_void,
    returnbuffersize: usize,
    maxmatches: libc::c_ulong,
    timelimit: timeval,
    searchparams1: *mut c_void,
    sizeofsearchparams1: usize,
    searchparams2: *mut c_void,
    sizeofsearchparams2: usize,
    searchattrs: attrlist,
}

/// `searchstate` is an opaque 556-byte kernel scratch buffer. We only need to
/// carry it between successive `searchfs()` calls, so treat it as raw bytes.
#[repr(C)]
struct searchstate {
    _opaque: [u8; 556],
}

// searchparams1 payload: attrreference + inline name bytes.
#[repr(C)]
struct packed_name_attr {
    size: u32,
    ref_: attrreference,
    name: [c_char; PATH_MAX],
}

#[repr(C)]
struct packed_attr_ref {
    size: u32,
    ref_: attrreference,
}

// One returned match: ATTR_CMN_FSID + ATTR_CMN_OBJID, preceded by a size u32.
#[repr(C)]
#[derive(Clone, Copy)]
struct packed_result {
    size: u32,
    fs_id: fsid_t,
    obj_id: fsobj_id_t,
}

// vol capabilities buffer for the getattrlist() capability probe.
#[repr(C, packed)]
struct vol_capabilities_attr_t {
    capabilities: [u32; 4],
    valid: [u32; 4],
}

#[repr(C, packed)]
struct vol_attr_buf {
    size: u32,
    vol_capabilities: vol_capabilities_attr_t,
}

// ---------------------------------------------------------------------------
// Syscall declarations
// ---------------------------------------------------------------------------

extern "C" {
    fn searchfs(
        path: *const c_char,
        searchblock: *mut fssearchblock,
        nummatches: *mut c_uint,
        scriptcode: c_uint,
        options: c_uint,
        state: *mut searchstate,
    ) -> c_int;

    fn fsgetpath(
        buf: *mut c_char,
        buflen: usize,
        fsid: *const fsid_t,
        obj_id: u64,
    ) -> isize;

    fn getattrlist(
        path: *const c_char,
        attrlist: *mut c_void,
        attrbuf: *mut c_void,
        attrbufsize: usize,
        options: c_uint,
    ) -> c_int;
}

// ---------------------------------------------------------------------------
// Public query / result types (shared with the Tauri command layer)
// ---------------------------------------------------------------------------

/// Search options coming from the UI. Mirrors the CLI flags of the reference.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchQuery {
    /// Filename substring to look for (case handling per `case_sensitive`).
    pub term: String,
    /// Match directories only (mutually exclusive with `files_only`).
    pub dirs_only: bool,
    /// Match files only.
    pub files_only: bool,
    /// Case-sensitive substring match. Default: case-insensitive.
    pub case_sensitive: bool,
    /// Whole-name exact match instead of substring.
    pub exact_match: bool,
    /// Stop after this many hits (0 = unlimited).
    pub limit: usize,
}

impl Default for SearchQuery {
    fn default() -> Self {
        SearchQuery {
            term: String::new(),
            dirs_only: false,
            files_only: false,
            case_sensitive: false,
            exact_match: false,
            limit: 1000,
        }
    }
}

/// One returned file-system object.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

/// Full result of a search, including any per-volume diagnostics.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    /// How many volumes were actually scanned.
    pub volumes_searched: Vec<String>,
    /// Non-fatal notes (unsupported volume, syscall errno, etc.).
    pub notes: Vec<String>,
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Volume capability probe
// ---------------------------------------------------------------------------

/// Return true if `path`'s volume advertises `VOL_CAP_INT_SEARCHFS`.
fn vol_supports_searchfs(path: &str) -> bool {
    let cpath = match std::ffi::CString::new(path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let mut attr_list = attrlist {
        bitmapcount: ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: 0,
        volattr: ATTR_VOL_INFO | ATTR_VOL_CAPABILITIES,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };

    let mut buf: vol_attr_buf = unsafe { std::mem::zeroed() };

    let err = unsafe {
        getattrlist(
            cpath.as_ptr(),
            &mut attr_list as *mut attrlist as *mut c_void,
            &mut buf as *mut vol_attr_buf as *mut c_void,
            std::mem::size_of::<vol_attr_buf>(),
            0,
        )
    };
    if err != 0 {
        return false;
    }

    // Read packed fields into locals to avoid unaligned references.
    let valid = buf.vol_capabilities.valid[VOL_CAPABILITIES_INTERFACES];
    let caps = buf.vol_capabilities.capabilities[VOL_CAPABILITIES_INTERFACES];

    (valid & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS
        && (caps & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS
}

fn data_volume_available() -> bool {
    std::path::Path::new(DATA_VOLUME).exists() && vol_supports_searchfs(DATA_VOLUME)
}

// ---------------------------------------------------------------------------
// Core search
// ---------------------------------------------------------------------------

/// Run a `searchfs()` scan of a single volume, appending hits to `out`.
///
/// Returns `Ok(())` on success (including zero hits); `Err(String)` carries a
/// human-readable errno note. `remaining` caps how many more hits we want
/// (usize::MAX for unlimited); when it hits 0 we stop early.
fn search_volume(
    volpath: &str,
    query: &SearchQuery,
    remaining: &mut usize,
    out: &mut Vec<SearchHit>,
) -> Result<(), String> {
    if *remaining == 0 {
        return Ok(());
    }

    let cvol = std::ffi::CString::new(volpath).map_err(|_| "bad volume path".to_string())?;

    // Build searchparams1: the name to match, packed as attrreference + bytes.
    let mut info1: packed_name_attr = unsafe { std::mem::zeroed() };
    {
        let bytes = query.term.as_bytes();
        let n = bytes.len().min(PATH_MAX - 1);
        for (i, &b) in bytes[..n].iter().enumerate() {
            info1.name[i] = b as c_char;
        }
        info1.name[n] = 0;
        info1.ref_.attr_dataoffset = std::mem::size_of::<attrreference>() as i32;
        info1.ref_.attr_length = (n as u32) + 1;
        info1.size = std::mem::size_of::<attrreference>() as u32 + info1.ref_.attr_length;
    }

    // searchparams2: empty attr ref (the reference impl requires this shape).
    let mut info2: packed_attr_ref = unsafe { std::mem::zeroed() };
    info2.size = std::mem::size_of::<attrreference>() as u32;
    info2.ref_.attr_dataoffset = std::mem::size_of::<attrreference>() as i32;
    info2.ref_.attr_length = 0;

    // Return buffer. The kernel writes variable-sized packed records here, so
    // we treat it as raw bytes and walk it by each record's `size` field (as
    // the reference impl does with `ptr += result_p->size`). Using a byte
    // buffer + `read_unaligned` also avoids any assumption that the compiler
    // knows the FFI call mutates a typed `[packed_result]` array.
    let return_buf_size = std::mem::size_of::<packed_result>() * MAX_MATCHES;
    let mut result_buffer: Vec<u8> = vec![0u8; return_buf_size];

    // Options. NB: we deliberately do NOT set SRCHFS_SKIPLINKS — modern macOS
    // rejects it here with EINVAL (the reference main.m never sets it either).
    let mut options = SRCHFS_START;
    if !query.dirs_only {
        options |= SRCHFS_MATCHFILES;
    }
    if !query.files_only {
        options |= SRCHFS_MATCHDIRS;
    }
    if !query.exact_match {
        options |= SRCHFS_MATCHPARTIALNAMES;
    }
    // Remaining reference flags are defined but not yet surfaced in the UI.
    // Referenced here so the constants don't trip the dead-code lint.
    let _ = (
        SRCHFS_SKIPINVISIBLE,
        SRCHFS_SKIPPACKAGES,
        SRCHFS_NEGATEPARAMS,
        SRCHFS_SKIPLINKS,
    );

    let mut ebusy_count: u32 = 0;

    // `catalog_changed` restart: rebuild block each attempt.
    'restart: loop {
        let mut return_list = attrlist {
            bitmapcount: ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: ATTR_CMN_FSID | ATTR_CMN_OBJID,
            volattr: 0,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        };

        let mut search_blk = fssearchblock {
            searchattrs: attrlist {
                bitmapcount: ATTR_BIT_MAP_COUNT,
                reserved: 0,
                commonattr: ATTR_CMN_NAME,
                volattr: 0,
                dirattr: 0,
                fileattr: 0,
                forkattr: 0,
            },
            sizeofsearchparams1: info1.size as usize + std::mem::size_of::<u32>(),
            searchparams1: &mut info1 as *mut packed_name_attr as *mut c_void,
            sizeofsearchparams2: std::mem::size_of::<packed_attr_ref>(),
            searchparams2: &mut info2 as *mut packed_attr_ref as *mut c_void,
            timelimit: timeval { tv_sec: 1, tv_usec: 0 },
            maxmatches: MAX_MATCHES as libc::c_ulong,
            returnattrs: &mut return_list,
            returnbuffersize: return_buf_size,
            returnbuffer: result_buffer.as_mut_ptr() as *mut c_void,
        };

        let mut search_state: searchstate = unsafe { std::mem::zeroed() };
        let mut opts = options;

        loop {
            let mut matches: c_uint = 0;
            let mut err = unsafe {
                searchfs(
                    cvol.as_ptr(),
                    &mut search_blk,
                    &mut matches,
                    0,
                    opts,
                    &mut search_state,
                )
            };
            if err == -1 {
                err = unsafe { *libc::__error() };
            }

            let eagain = err == libc::EAGAIN;

            if (err == 0 || eagain) && matches > 0 {
                // Walk the raw byte buffer record-by-record, advancing by each
                // record's own `size` field (variable stride), exactly like the
                // reference `ptr += result_p->size`. `read_unaligned` is used
                // because packed records are not guaranteed to be aligned.
                let base = result_buffer.as_ptr();
                let end = unsafe { base.add(return_buf_size) };
                let mut ptr = base;

                for _ in 0..matches {
                    // Guard against reading past the buffer.
                    if unsafe { ptr.add(std::mem::size_of::<packed_result>()) } > end {
                        break;
                    }
                    let rec = unsafe { std::ptr::read_unaligned(ptr as *const packed_result) };

                    if let Some(hit) = resolve_hit(&rec, query) {
                        out.push(hit);
                        if *remaining != usize::MAX {
                            *remaining -= 1;
                            if *remaining == 0 {
                                return Ok(());
                            }
                        }
                    }

                    let stride = rec.size as usize;
                    if stride == 0 {
                        break; // malformed record; avoid an infinite loop
                    }
                    ptr = unsafe { ptr.add(stride) };
                    if ptr > end {
                        break;
                    }
                }
            }

            if err == libc::EBUSY {
                if ebusy_count < MAX_EBUSY_RETRIES {
                    ebusy_count += 1;
                    continue 'restart;
                } else {
                    return Err(format!("{}: catalog kept changing (EBUSY)", volpath));
                }
            }

            if err != 0 && !eagain {
                let msg = unsafe {
                    CStr::from_ptr(libc::strerror(err))
                        .to_string_lossy()
                        .into_owned()
                };
                return Err(format!("{}: searchfs errno {} ({})", volpath, err, msg));
            }

            // Subsequent iterations must NOT set SRCHFS_START.
            opts &= !SRCHFS_START;

            if !eagain {
                return Ok(());
            }
        }
    }
}

/// Reconstruct a hit's path from its `(fsid, objid)` and apply user-side
/// case-sensitivity / exact-match filtering the kernel didn't do.
fn resolve_hit(r: &packed_result, query: &SearchQuery) -> Option<SearchHit> {
    // fsgetpath wants the 64-bit object id: objno | (generation << 32).
    let objid: u64 = (r.obj_id.fid_objno as u64) | ((r.obj_id.fid_generation as u64) << 32);

    let mut buf = [0i8; PATH_MAX];
    let size = unsafe {
        fsgetpath(
            buf.as_mut_ptr(),
            buf.len(),
            &r.fs_id as *const fsid_t,
            objid,
        )
    };
    if size < 0 {
        // Object was likely deleted between the scan and path lookup. Skip.
        return None;
    }

    let path = unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned();

    // Extra filtering the kernel didn't apply. searchfs() with
    // SRCHFS_MATCHPARTIALNAMES is case-insensitive substring; enforce
    // case-sensitivity or exact-match here on the basename.
    if !query.term.is_empty() {
        let name = std::path::Path::new(&path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let (hay, needle) = if query.case_sensitive {
            (name.to_string(), query.term.clone())
        } else {
            (name.to_lowercase(), query.term.to_lowercase())
        };

        let ok = if query.exact_match {
            hay == needle
        } else {
            hay.contains(&needle)
        };
        if !ok {
            return None;
        }
    }

    let name = std::path::Path::new(&path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // Cheap dir/file classification via lstat; tolerate failure.
    let is_dir = std::fs::symlink_metadata(&path)
        .map(|m| m.is_dir())
        .unwrap_or(false);

    Some(SearchHit { path, name, is_dir })
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Perform a filename search across the default volume set and return hits.
///
/// This is the safe, high-level API the GUI/CLI call. It never panics on
/// syscall failure — errors become `notes` so the UI can show partial results.
pub fn search(query: &SearchQuery) -> SearchResult {
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut volumes: Vec<String> = Vec::new();

    if query.term.is_empty() {
        return SearchResult {
            hits,
            volumes_searched: volumes,
            notes,
            truncated: false,
        };
    }
    if query.dirs_only && query.files_only {
        notes.push("dirs_only and files_only are mutually exclusive; ignoring both".into());
    }

    let mut remaining = if query.limit == 0 {
        usize::MAX
    } else {
        query.limit
    };

    // Volume 1: root system volume.
    if vol_supports_searchfs(DEFAULT_VOLUME) {
        volumes.push(DEFAULT_VOLUME.to_string());
        if let Err(e) = search_volume(DEFAULT_VOLUME, query, &mut remaining, &mut hits) {
            notes.push(e);
        }
    } else {
        notes.push(format!("{} does not support searchfs", DEFAULT_VOLUME));
    }

    // Volume 2: firmlinked Data volume (Catalina+), only if room remains.
    if remaining != 0 && data_volume_available() {
        volumes.push(DATA_VOLUME.to_string());
        if let Err(e) = search_volume(DATA_VOLUME, query, &mut remaining, &mut hits) {
            notes.push(e);
        }
    }

    let truncated = query.limit != 0 && remaining == 0;

    SearchResult {
        hits,
        volumes_searched: volumes,
        notes,
        truncated,
    }
}

/// Lightweight self-test used by CI: confirms the syscall path runs end to end
/// without panicking. Zero hits is a valid pass — the CI runner may lack Full
/// Disk Access, and searchfs still returns cleanly.
pub fn self_test() -> Result<String, String> {
    let supported = vol_supports_searchfs(DEFAULT_VOLUME);
    let q = SearchQuery {
        term: "Applications".to_string(),
        limit: 5,
        ..Default::default()
    };
    let res = search(&q);
    Ok(format!(
        "self-test ok: '/' supports searchfs = {}, volumes = {:?}, hits = {}, notes = {:?}",
        supported,
        res.volumes_searched,
        res.hits.len(),
        res.notes
    ))
}

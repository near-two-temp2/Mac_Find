//! Raw FFI declarations for the macOS `searchfs(2)` catalog search syscall and
//! the private `fsgetpath` SPI used to turn (fsid, objid) pairs back into paths.
//!
//! `libc` does not ship any of this, so we declare the structs, constants and
//! `extern "C"` prototypes here by hand, matching `<sys/attr.h>`,
//! `<sys/vnode.h>` and `<sys/fsgetpath.h>`. Layout mirrors the reference
//! Objective-C implementation in `../Open_Ref/searchfs/main.m`.
#![cfg(target_os = "macos")]
#![allow(non_camel_case_types)]

// NOTE: `libc` does NOT export the BSD `u_int32_t` alias, so we use plain `u32`
// (the ABI-identical type) everywhere the C headers say `u_int32_t`.
use libc::{c_char, c_int, c_uint, c_void, timeval};

/// `PATH_MAX` on Darwin.
pub const PATH_MAX: usize = 1024;

/// `<sys/attr.h>`: number of 32-bit words in an attribute bitmap.
pub const ATTR_BIT_MAP_COUNT: u16 = 5;

// --- Common attribute flags (`ATTR_CMN_*`) we care about. ---
pub const ATTR_CMN_NAME: u32 = 0x0000_0001;
pub const ATTR_CMN_OBJID: u32 = 0x0000_0020;
pub const ATTR_CMN_FSID: u32 = 0x0000_0040;

// --- Volume attribute / capability flags used to probe searchfs support. ---
pub const ATTR_VOL_INFO: u32 = 0x8000_0000;
pub const ATTR_VOL_CAPABILITIES: u32 = 0x0002_0000;

/// Index into the `vol_capabilities_attr_t` arrays for the interface set.
pub const VOL_CAPABILITIES_INTERFACES: usize = 1;
/// `VOL_CAP_INT_SEARCHFS` — volume advertises searchfs support.
pub const VOL_CAP_INT_SEARCHFS: u32 = 0x0000_0001;

// --- searchfs() option flags (`SRCHFS_*`, `<sys/attr.h>`). ---
pub const SRCHFS_START: c_uint = 0x0000_0001;
pub const SRCHFS_MATCHPARTIALNAMES: c_uint = 0x0000_0002;
pub const SRCHFS_MATCHDIRS: c_uint = 0x0000_0004;
pub const SRCHFS_MATCHFILES: c_uint = 0x0000_0008;
pub const SRCHFS_SKIPLINKS: c_uint = 0x0000_0010;
pub const SRCHFS_SKIPINVISIBLE: c_uint = 0x0000_0020;
pub const SRCHFS_SKIPPACKAGES: c_uint = 0x0000_0040;
pub const SRCHFS_NEGATEPARAMS: c_uint = 0x0000_0080;

/// `struct attrlist` — which attributes a caller wants to search on / receive.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct attrlist {
    pub bitmapcount: u16,
    pub reserved: u16,
    pub commonattr: u32,
    pub volattr: u32,
    pub dirattr: u32,
    pub fileattr: u32,
    pub forkattr: u32,
}

impl Default for attrlist {
    fn default() -> Self {
        attrlist {
            bitmapcount: ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: 0,
            volattr: 0,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        }
    }
}

/// `struct attrreference` — offset+length pointer into a packed attr buffer.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct attrreference {
    pub attr_dataoffset: i32,
    pub attr_length: u32,
}

/// `struct fssearchblock` — the search descriptor handed to searchfs().
#[repr(C)]
pub struct fssearchblock {
    pub returnattrs: *mut attrlist,
    pub returnbuffer: *mut c_void,
    pub returnbuffersize: usize,
    pub maxmatches: usize,
    pub timelimit: timeval,
    pub searchparams1: *mut c_void,
    pub sizeofsearchparams1: usize,
    pub searchparams2: *mut c_void,
    pub sizeofsearchparams2: usize,
    pub searchattrs: attrlist,
}

/// `struct searchstate` — opaque kernel state carried across resumed calls.
/// Declared as a fixed byte blob (`__darwin_size_t state[556 / sizeof...]`);
/// 556 bytes is the documented size on Darwin.
#[repr(C)]
pub struct searchstate {
    pub reserved: [u8; 556],
}

impl Default for searchstate {
    fn default() -> Self {
        searchstate { reserved: [0u8; 556] }
    }
}

/// `struct fsid { int32_t val[2]; }`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct fsid_t {
    pub val: [i32; 2],
}

/// `struct fsobj_id { u_int32_t fid_objno; u_int32_t fid_generation; }`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct fsobj_id_t {
    pub fid_objno: u32,
    pub fid_generation: u32,
}

/// Returned-attribute record layout for each match: leading size word (from
/// `ATTR_BIT_MAP_COUNT` return attrs) followed by fsid + objid. Mirrors
/// `struct packed_result` in the reference C.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct packed_result {
    pub size: u32,
    pub fs_id: fsid_t,
    pub obj_id: fsobj_id_t,
}

/// `vol_capabilities_attr_t` — capability + validity bit arrays returned by
/// getattrlist() when probing `ATTR_VOL_CAPABILITIES`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct vol_capabilities_attr {
    pub capabilities: [u32; 4],
    pub valid: [u32; 4],
}

extern "C" {
    /// `int searchfs(const char *path, struct fssearchblock *searchBlock,
    ///               unsigned long *nummatches, unsigned int scriptcode,
    ///               unsigned int options, struct searchstate *state);`
    pub fn searchfs(
        path: *const c_char,
        search_block: *mut fssearchblock,
        num_matches: *mut libc::c_ulong,
        script_code: c_uint,
        options: c_uint,
        state: *mut searchstate,
    ) -> c_int;

    /// `ssize_t fsgetpath(char *restrict buf, size_t bufsize,
    ///                    fsid_t *fsid, uint64_t obj_id);` (private SPI)
    pub fn fsgetpath(
        buf: *mut c_char,
        bufsize: libc::size_t,
        fsid: *const fsid_t,
        obj_id: u64,
    ) -> libc::ssize_t;

    /// `int getattrlist(const char *path, struct attrlist *attrList,
    ///                  void *attrBuf, size_t attrBufSize, unsigned long options);`
    pub fn getattrlist(
        path: *const c_char,
        attr_list: *mut attrlist,
        attr_buf: *mut c_void,
        attr_buf_size: libc::size_t,
        options: libc::c_ulong,
    ) -> c_int;
}

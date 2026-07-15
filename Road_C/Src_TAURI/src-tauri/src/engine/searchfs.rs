//! Fallback engine: live filename search via the macOS `searchfs(2)` syscall.
//!
//! This is a direct Rust FFI port of the calling sequence in
//! `Open_Ref/searchfs/main.m`. It walks the filesystem catalog (APFS/HFS+
//! B-Tree) in the kernel — roughly 100x faster than `find` — and never
//! touches a user-space index. Road_C uses it as the safety net when the
//! self-built mmap index is missing or corrupt.
//!
//! Only compiled on macOS. On other targets the public functions return an
//! error so the crate still builds (useful for `cargo check` on CI matrix
//! sanity, though the authoritative build is macos-latest).

use crate::engine::types::{Hit, SearchOptions};

/// Search both the system volume `/` and the data volume
/// `/System/Volumes/Data` (Catalina+ split), de-duplicating results.
/// Stops once `opts.limit` hits are collected.
pub fn search(query: &str, opts: &SearchOptions) -> Result<Vec<Hit>, String> {
    #[cfg(target_os = "macos")]
    {
        imp::search(query, opts)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (query, opts);
        Err("searchfs() is only available on macOS".into())
    }
}

/// Cheap probe: does the default volume support catalog search? Used by the
/// GUI to report engine availability without running a full search.
pub fn is_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        imp::vol_supports_searchfs("/")
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::collections::HashSet;
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int, c_uint, c_ulong, c_void};

    // ---- libc / kernel constants (from <sys/attr.h>, <sys/vnode.h>) --------

    const ATTR_BIT_MAP_COUNT: u16 = 5;
    const ATTR_CMN_NAME: u32 = 0x0000_0001;
    const ATTR_CMN_FSID: u32 = 0x0000_0004;
    const ATTR_CMN_OBJID: u32 = 0x0000_0020;

    const ATTR_VOL_INFO: u32 = 0x8000_0000;
    const ATTR_VOL_CAPABILITIES: u32 = 0x0002_0000;

    // Values verified against <sys/attr.h> in the macOS SDK.
    const SRCHFS_START: c_uint = 0x0000_0001;
    const SRCHFS_MATCHPARTIALNAMES: c_uint = 0x0000_0002;
    const SRCHFS_MATCHDIRS: c_uint = 0x0000_0004;
    const SRCHFS_MATCHFILES: c_uint = 0x0000_0008;
    const SRCHFS_SKIPINVISIBLE: c_uint = 0x0000_0020;
    const SRCHFS_SKIPPACKAGES: c_uint = 0x0000_0040;

    const VOL_CAPABILITIES_INTERFACES: usize = 1;
    const VOL_CAP_INT_SEARCHFS: u32 = 0x0000_0001;

    const EAGAIN: c_int = 35;
    const EBUSY: c_int = 16;
    const MAX_MATCHES: usize = 32;
    const MAX_EBUSY_RETRIES: u32 = 5;
    const PATH_MAX: usize = 1024;

    // ---- kernel structs ---------------------------------------------------

    #[repr(C)]
    struct AttrList {
        bitmapcount: u16,
        reserved: u16,
        commonattr: u32,
        volattr: u32,
        dirattr: u32,
        fileattr: u32,
        forkattr: u32,
    }

    #[repr(C)]
    struct AttrReference {
        attr_dataoffset: i32,
        attr_length: u32,
    }

    #[repr(C)]
    struct Timeval {
        tv_sec: i64,
        tv_usec: i32,
    }

    #[repr(C)]
    struct Fsid {
        val: [i32; 2],
    }

    #[repr(C)]
    struct FsobjId {
        fid_objno: u32,
        fid_generation: u32,
    }

    #[repr(C)]
    struct FsSearchBlock {
        returnattrs: *mut AttrList,
        returnbuffer: *mut c_void,
        returnbuffersize: usize,
        // NOTE: `u_long` in the C struct — 8 bytes on 64-bit. Using c_uint here
        // would misalign every field after it and corrupt the syscall args.
        maxmatches: c_ulong,
        timelimit: Timeval,
        searchparams1: *mut c_void,
        sizeofsearchparams1: usize,
        searchparams2: *mut c_void,
        sizeofsearchparams2: usize,
        searchattrs: AttrList,
    }

    // struct searchstate is __attribute__((packed)): 4 + 4 + 548 = 556 bytes.
    #[repr(C, packed)]
    struct SearchState {
        ss_union_flags: u32,
        ss_union_layer: u32,
        ss_fsstate: [u8; 548],
    }

    #[repr(C)]
    struct PackedNameAttr {
        size: u32,
        ref_: AttrReference,
        name: [c_char; PATH_MAX],
    }

    #[repr(C)]
    struct PackedAttrRef {
        size: u32,
        ref_: AttrReference,
    }

    #[repr(C)]
    struct PackedResult {
        size: u32,
        fs_id: Fsid,
        obj_id: FsobjId,
    }

    #[repr(C, packed(4))]
    struct VolCapabilitiesAttr {
        capabilities: [u32; 4],
        valid: [u32; 4],
    }

    #[repr(C, packed(4))]
    struct VolAttrBuf {
        size: u32,
        vol_capabilities: VolCapabilitiesAttr,
    }

    // ---- extern syscalls --------------------------------------------------

    extern "C" {
        fn searchfs(
            path: *const c_char,
            searchblock: *mut FsSearchBlock,
            nummatches: *mut c_ulong,
            scriptcode: c_uint,
            options: c_uint,
            state: *mut SearchState,
        ) -> c_int;

        fn getattrlist(
            path: *const c_char,
            attrlist: *mut AttrList,
            attr_buf: *mut c_void,
            attr_buf_size: usize,
            options: c_uint,
        ) -> c_int;

        fn fsgetpath(
            buf: *mut c_char,
            bufsize: usize,
            fsid: *const Fsid,
            objid: u64,
        ) -> isize;

        fn __error() -> *mut c_int;
    }

    fn errno() -> c_int {
        unsafe { *__error() }
    }

    pub fn vol_supports_searchfs(path: &str) -> bool {
        let c = match CString::new(path) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let mut attr_list = AttrList {
            bitmapcount: ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: 0,
            volattr: ATTR_VOL_INFO | ATTR_VOL_CAPABILITIES,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        };
        let mut buf = VolAttrBuf {
            size: 0,
            vol_capabilities: VolCapabilitiesAttr {
                capabilities: [0; 4],
                valid: [0; 4],
            },
        };
        let rc = unsafe {
            getattrlist(
                c.as_ptr(),
                &mut attr_list,
                &mut buf as *mut _ as *mut c_void,
                std::mem::size_of::<VolAttrBuf>(),
                0,
            )
        };
        if rc != 0 {
            return false;
        }
        let caps = &buf.vol_capabilities;
        (caps.valid[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS
            && (caps.capabilities[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS)
                == VOL_CAP_INT_SEARCHFS
    }

    pub fn search(query: &str, opts: &SearchOptions) -> Result<Vec<Hit>, String> {
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut out: Vec<Hit> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        // System volume, then the Catalina+ data volume.
        search_volume("/", query, opts, &mut out, &mut seen)?;
        if opts.limit == 0 || out.len() < opts.limit {
            let data = "/System/Volumes/Data";
            if std::path::Path::new(data).exists() && vol_supports_searchfs(data) {
                search_volume(data, query, opts, &mut out, &mut seen)?;
            }
        }
        Ok(out)
    }

    fn search_volume(
        volpath: &str,
        query: &str,
        opts: &SearchOptions,
        out: &mut Vec<Hit>,
        seen: &mut HashSet<String>,
    ) -> Result<(), String> {
        let vol_c = CString::new(volpath).map_err(|e| e.to_string())?;

        let mut ebusy_count: u32 = 0;
        'restart: loop {
            let mut return_list = AttrList {
                bitmapcount: ATTR_BIT_MAP_COUNT,
                reserved: 0,
                commonattr: ATTR_CMN_FSID | ATTR_CMN_OBJID,
                volattr: 0,
                dirattr: 0,
                fileattr: 0,
                forkattr: 0,
            };

            // Pack searchparams1 (the name to match). See main.m:311-318.
            let name_bytes = query.as_bytes();
            let mut info1 = PackedNameAttr {
                size: 0,
                ref_: AttrReference {
                    attr_dataoffset: std::mem::size_of::<AttrReference>() as i32,
                    attr_length: (name_bytes.len() + 1) as u32,
                },
                name: [0; PATH_MAX],
            };
            for (i, &b) in name_bytes.iter().enumerate().take(PATH_MAX - 1) {
                info1.name[i] = b as c_char;
            }
            info1.size =
                (std::mem::size_of::<AttrReference>() as u32) + info1.ref_.attr_length;

            let mut info2 = PackedAttrRef {
                size: std::mem::size_of::<AttrReference>() as u32,
                ref_: AttrReference {
                    attr_dataoffset: std::mem::size_of::<AttrReference>() as i32,
                    attr_length: 0,
                },
            };

            let mut result_buffer = vec![0u8; MAX_MATCHES * std::mem::size_of::<PackedResult>()];

            let mut search_blk = FsSearchBlock {
                returnattrs: &mut return_list,
                returnbuffer: result_buffer.as_mut_ptr() as *mut c_void,
                returnbuffersize: result_buffer.len(),
                maxmatches: MAX_MATCHES as c_ulong,
                timelimit: Timeval {
                    tv_sec: 1,
                    tv_usec: 0,
                },
                searchparams1: &mut info1 as *mut _ as *mut c_void,
                sizeofsearchparams1: (info1.size as usize) + std::mem::size_of::<u32>(),
                searchparams2: &mut info2 as *mut _ as *mut c_void,
                sizeofsearchparams2: std::mem::size_of::<PackedAttrRef>(),
                searchattrs: AttrList {
                    bitmapcount: ATTR_BIT_MAP_COUNT,
                    reserved: 0,
                    commonattr: ATTR_CMN_NAME,
                    volattr: 0,
                    dirattr: 0,
                    fileattr: 0,
                    forkattr: 0,
                },
            };

            let mut options: c_uint = SRCHFS_START;
            if !opts.dirs_only {
                options |= SRCHFS_MATCHFILES;
            }
            if !opts.files_only {
                options |= SRCHFS_MATCHDIRS;
            }
            options |= SRCHFS_MATCHPARTIALNAMES;
            if opts.skip_packages {
                options |= SRCHFS_SKIPPACKAGES;
            }
            if opts.skip_invisibles {
                options |= SRCHFS_SKIPINVISIBLE;
            }

            let mut state = SearchState {
                ss_union_flags: 0,
                ss_union_layer: 0,
                ss_fsstate: [0; 548],
            };

            loop {
                let mut num_matches: c_ulong = 0;
                let mut err = unsafe {
                    searchfs(
                        vol_c.as_ptr(),
                        &mut search_blk,
                        &mut num_matches,
                        0,
                        options,
                        &mut state,
                    )
                };
                if err == -1 {
                    err = errno();
                }

                if (err == 0 || err == EAGAIN) && num_matches > 0 {
                    let base = result_buffer.as_ptr();
                    let mut ptr = base;
                    let end = unsafe { base.add(result_buffer.len()) };
                    for _ in 0..num_matches {
                        let result = ptr as *const PackedResult;
                        let rsize = unsafe { (*result).size } as usize;

                        let mut path_buf = [0u8; PATH_MAX];
                        let fs_id_ptr = unsafe { &(*result).fs_id as *const Fsid };
                        let objid = unsafe {
                            (*result).obj_id.fid_objno as u64
                                | (((*result).obj_id.fid_generation as u64) << 32)
                        };
                        let sz = unsafe {
                            fsgetpath(
                                path_buf.as_mut_ptr() as *mut c_char,
                                PATH_MAX,
                                fs_id_ptr,
                                objid,
                            )
                        };
                        if sz > 0 {
                            let cstr =
                                unsafe { CStr::from_ptr(path_buf.as_ptr() as *const c_char) };
                            if let Ok(path) = cstr.to_str() {
                                if filter(path, query, opts) && seen.insert(path.to_string()) {
                                    out.push(Hit::from_path(path, 0));
                                    if opts.limit != 0 && out.len() >= opts.limit {
                                        return Ok(());
                                    }
                                }
                            }
                        }

                        ptr = unsafe { ptr.add(rsize) };
                        if ptr > end {
                            break;
                        }
                    }
                }

                if err == EBUSY {
                    if ebusy_count < MAX_EBUSY_RETRIES {
                        ebusy_count += 1;
                        continue 'restart;
                    } else {
                        return Ok(());
                    }
                }

                if err != 0 && err != EAGAIN {
                    // Permission failures (Full Disk Access) are common on CI;
                    // surface as an error only for the first volume so callers
                    // can fall through gracefully.
                    return Err(format!("searchfs() failed on {volpath}: errno {err}"));
                }

                options &= !SRCHFS_START;
                if err != EAGAIN {
                    break;
                }
            }
            return Ok(());
        }
    }

    /// Post-processing filter mirroring main.m: the kernel already did a
    /// case-insensitive substring match, so we only refine for case
    /// sensitivity and prefix/suffix anchors here.
    fn filter(path: &str, query: &str, opts: &SearchOptions) -> bool {
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);

        if opts.case_sensitive && !path.contains(query) {
            return false;
        }
        true
            && (!opts.match_start || name.starts_with(query))
            && (!opts.match_end || name.ends_with(query))
    }
}

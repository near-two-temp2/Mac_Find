//! Safe(ish) wrapper around the raw `searchfs(2)` FFI in [`crate::searchfs_sys`].
//!
//! This is the Road_A engine: no index, every query is a live catalog scan of
//! the mounted APFS/HFS+ volumes. It mirrors the control flow of the reference
//! `../Open_Ref/searchfs/main.m` — pack the name into `searchparams1`, request
//! `ATTR_CMN_FSID | ATTR_CMN_OBJID` back, loop on `EAGAIN`, retry a bounded
//! number of times on `EBUSY` (catalog changed mid-search), and resolve each
//! (fsid, objid) hit to a path with `fsgetpath`.
//!
//! On non-macOS targets the whole module compiles to a stub so the crate still
//! type-checks elsewhere; the real work only exists behind `cfg(macos)`.

/// One search hit: absolute path plus a cheap directory flag (best-effort).
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub path: String,
    pub is_dir: bool,
}

/// User-facing search parameters, wired up from the GUI controls / CLI flags.
#[derive(Clone, Debug)]
pub struct SearchOptions {
    /// The substring (or exact name) to look for.
    pub query: String,
    /// Match directories only.
    pub dirs_only: bool,
    /// Match files only.
    pub files_only: bool,
    /// Exact filename match instead of substring.
    pub exact_match: bool,
    /// Case-sensitive matching. `searchfs` matches case-insensitively at the
    /// kernel level, so this is enforced by a post-filter.
    pub case_sensitive: bool,
    /// Stop after this many hits (0 = unlimited).
    pub limit: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions {
            query: String::new(),
            dirs_only: false,
            files_only: false,
            exact_match: false,
            case_sensitive: false,
            limit: 5000,
        }
    }
}

/// The two volumes we scan by default on Catalina+ (read-only system volume `/`
/// and the writable data volume). Matches the reference implementation.
pub const DEFAULT_VOLUME: &str = "/";
pub const DATA_VOLUME: &str = "/System/Volumes/Data";

#[cfg(target_os = "macos")]
mod imp {
    use super::{SearchHit, SearchOptions, DATA_VOLUME, DEFAULT_VOLUME};
    use crate::searchfs_sys::*;
    use libc::{c_ulong, timeval, EAGAIN, EBUSY};
    use std::ffi::CString;
    use std::mem;
    use std::path::Path;

    /// Matches the C `#define MAX_MATCHES` / `MAX_EBUSY_RETRIES`.
    const MAX_MATCHES: usize = 256;
    const MAX_EBUSY_RETRIES: u32 = 5;

    /// Packed `ATTR_CMN_NAME` search param buffer: `struct packed_name_attr`.
    #[repr(C)]
    struct PackedNameAttr {
        size: u32,
        ref_: attrreference,
        name: [u8; PATH_MAX],
    }

    /// Packed empty second search param buffer: `struct packed_attr_ref`.
    #[repr(C)]
    struct PackedAttrRef {
        size: u32,
        ref_: attrreference,
    }

    /// Probe whether `path` is a mounted volume that advertises searchfs.
    pub fn volume_supports_searchfs(path: &str) -> bool {
        #[repr(C, packed(4))]
        struct VolAttrBuf {
            size: u32,
            caps: vol_capabilities_attr,
        }

        let c_path = match CString::new(path) {
            Ok(p) => p,
            Err(_) => return false,
        };

        let mut attr_list = attrlist {
            volattr: ATTR_VOL_INFO | ATTR_VOL_CAPABILITIES,
            ..Default::default()
        };
        let mut buf: VolAttrBuf = unsafe { mem::zeroed() };

        let err = unsafe {
            getattrlist(
                c_path.as_ptr(),
                &mut attr_list,
                &mut buf as *mut _ as *mut libc::c_void,
                mem::size_of::<VolAttrBuf>(),
                0,
            )
        };
        if err != 0 {
            return false;
        }

        let valid = buf.caps.valid[VOL_CAPABILITIES_INTERFACES];
        let caps = buf.caps.capabilities[VOL_CAPABILITIES_INTERFACES];
        (valid & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS
            && (caps & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS
    }

    /// Whether the data volume exists and supports searchfs (Catalina+).
    fn data_volume_available() -> bool {
        Path::new(DATA_VOLUME).exists() && volume_supports_searchfs(DATA_VOLUME)
    }

    /// Post-filter a raw catalog hit. `searchfs` already did a case-insensitive
    /// substring match, so we only need to enforce the stricter options here:
    /// case sensitivity, exact match, and the dirs/files split (which we can't
    /// always express at the kernel level cleanly for every FS).
    fn passes_filters(path: &str, opts: &SearchOptions) -> bool {
        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);

        if opts.exact_match {
            return if opts.case_sensitive {
                name == opts.query
            } else {
                name.eq_ignore_ascii_case(&opts.query)
            };
        }

        if opts.case_sensitive {
            // Kernel matched case-insensitively; re-check the whole path
            // case-sensitively to honour the flag.
            return path.contains(&opts.query);
        }

        true
    }

    /// Run one searchfs pass against a single volume, appending hits.
    /// Returns the number of matches appended.
    fn search_one_volume(
        volpath: &str,
        opts: &SearchOptions,
        out: &mut Vec<SearchHit>,
    ) -> usize {
        let c_vol = match CString::new(volpath) {
            Ok(v) => v,
            Err(_) => return 0,
        };

        // Bound the query length to the name buffer.
        let query_bytes = opts.query.as_bytes();
        if query_bytes.is_empty() || query_bytes.len() >= PATH_MAX {
            return 0;
        }

        let mut appended = 0usize;
        let mut ebusy_count = 0u32;

        // Result buffer for the returned (size, fsid, objid) records.
        let mut result_buffer = [packed_result::default(); MAX_MATCHES];

        // ---- Build the (re-usable) search block. ----
        // searchparams1: packed name.
        let mut info1: PackedNameAttr = unsafe { mem::zeroed() };
        info1.name[..query_bytes.len()].copy_from_slice(query_bytes);
        info1.name[query_bytes.len()] = 0; // NUL terminate
        info1.ref_.attr_dataoffset = mem::size_of::<attrreference>() as i32;
        info1.ref_.attr_length = (query_bytes.len() + 1) as u32;
        info1.size = mem::size_of::<attrreference>() as u32 + info1.ref_.attr_length;

        // searchparams2: empty.
        let mut info2 = PackedAttrRef {
            size: mem::size_of::<attrreference>() as u32,
            ref_: attrreference {
                attr_dataoffset: mem::size_of::<attrreference>() as i32,
                attr_length: 0,
            },
        };

        let mut return_list = attrlist {
            commonattr: ATTR_CMN_FSID | ATTR_CMN_OBJID,
            ..Default::default()
        };

        let mut search_attrs = attrlist::default();
        search_attrs.commonattr = ATTR_CMN_NAME;

        // Assemble search options bitmask.
        let mut search_options: libc::c_uint = SRCHFS_START;
        if !opts.dirs_only {
            search_options |= SRCHFS_MATCHFILES;
        }
        if !opts.files_only {
            search_options |= SRCHFS_MATCHDIRS;
        }
        if !opts.exact_match {
            search_options |= SRCHFS_MATCHPARTIALNAMES;
        }

        // The `catalog_changed` restart label in C becomes a bounded loop here.
        loop {
            let mut restart = false;
            let mut state = searchstate::default();

            let mut block = fssearchblock {
                returnattrs: &mut return_list,
                returnbuffer: result_buffer.as_mut_ptr() as *mut libc::c_void,
                returnbuffersize: mem::size_of_val(&result_buffer),
                maxmatches: MAX_MATCHES,
                timelimit: timeval { tv_sec: 1, tv_usec: 0 },
                searchparams1: &mut info1 as *mut _ as *mut libc::c_void,
                sizeofsearchparams1: info1.size as usize + mem::size_of::<u32>(),
                searchparams2: &mut info2 as *mut _ as *mut libc::c_void,
                sizeofsearchparams2: mem::size_of::<PackedAttrRef>(),
                searchattrs: search_attrs,
            };

            let mut this_options = search_options;

            // Inner loop: keep calling while EAGAIN (more results pending).
            loop {
                let mut matches: c_ulong = 0;
                let mut err = unsafe {
                    searchfs(
                        c_vol.as_ptr(),
                        &mut block,
                        &mut matches,
                        0,
                        this_options,
                        &mut state,
                    )
                };
                if err == -1 {
                    err = errno();
                }

                if (err == 0 || err == EAGAIN) && matches > 0 {
                    unpack_results(
                        &result_buffer,
                        matches as usize,
                        opts,
                        out,
                        &mut appended,
                    );
                    if opts.limit != 0 && appended >= opts.limit {
                        return appended;
                    }
                }

                if err == EBUSY && ebusy_count < MAX_EBUSY_RETRIES {
                    ebusy_count += 1;
                    restart = true;
                    break; // restart the whole search
                }

                if err != 0 && err != EAGAIN {
                    // Genuine failure (e.g. EPERM without full-disk access).
                    return appended;
                }

                // Clear START bit so subsequent calls resume the search.
                this_options &= !SRCHFS_START;

                if err != EAGAIN {
                    return appended;
                }
            }

            if !restart {
                return appended;
            }
            // else loop back and restart from scratch (catalog changed).
        }
    }

    /// Walk the packed result buffer, resolve each hit to a path and filter.
    fn unpack_results(
        buffer: &[packed_result],
        matches: usize,
        opts: &SearchOptions,
        out: &mut Vec<SearchHit>,
        appended: &mut usize,
    ) {
        for r in buffer.iter().take(matches) {
            let obj_id: u64 =
                (r.obj_id.fid_objno as u64) | ((r.obj_id.fid_generation as u64) << 32);

            let mut path_buf = [0i8; PATH_MAX];
            let size = unsafe {
                fsgetpath(
                    path_buf.as_mut_ptr(),
                    PATH_MAX,
                    &r.fs_id,
                    obj_id,
                )
            };
            if size < 0 {
                // Object likely deleted between match and lookup; skip silently.
                continue;
            }

            let bytes = unsafe {
                std::slice::from_raw_parts(path_buf.as_ptr() as *const u8, size as usize)
            };
            let path = match std::str::from_utf8(bytes) {
                Ok(s) => s.to_string(),
                Err(_) => String::from_utf8_lossy(bytes).into_owned(),
            };

            if !passes_filters(&path, opts) {
                continue;
            }

            let is_dir = std::fs::metadata(&path)
                .map(|m| m.is_dir())
                .unwrap_or(false);

            out.push(SearchHit { path, is_dir });
            *appended += 1;
            if opts.limit != 0 && *appended >= opts.limit {
                return;
            }
        }
    }

    /// Read `errno` in a portable way.
    fn errno() -> i32 {
        unsafe { *libc::__error() }
    }

    /// Public entry point: search all default volumes and return hits.
    pub fn search(opts: &SearchOptions) -> Vec<SearchHit> {
        let mut out = Vec::new();
        if opts.query.is_empty() {
            return out;
        }

        // Root volume first.
        search_one_volume(DEFAULT_VOLUME, opts, &mut out);

        // Then the data volume on Catalina+, respecting the limit.
        let want_more = opts.limit == 0 || out.len() < opts.limit;
        if want_more && data_volume_available() {
            let mut data_opts = opts.clone();
            if opts.limit != 0 {
                data_opts.limit = opts.limit.saturating_sub(out.len());
            }
            search_one_volume(DATA_VOLUME, &data_opts, &mut out);
        }

        out
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::{SearchHit, SearchOptions};

    /// Non-macOS stub: searchfs does not exist, so return nothing. Keeps the
    /// crate buildable on Linux/CI-lint hosts even though the real target is
    /// always macos-latest.
    pub fn search(_opts: &SearchOptions) -> Vec<SearchHit> {
        Vec::new()
    }

    pub fn volume_supports_searchfs(_path: &str) -> bool {
        false
    }
}

pub use imp::{search, volume_supports_searchfs};

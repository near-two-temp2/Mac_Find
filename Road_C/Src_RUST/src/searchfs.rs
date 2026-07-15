//! `searchfs(2)` 实时兜底引擎 —— 混合引擎的降级路径。
//!
//! 当自建索引缺失 / 损坏 / 尚未建立时，混合引擎回退到这里：直接调用 macOS
//! 内核 `searchfs()` 在 APFS/HFS+ catalog 上实时子串匹配文件名，用私有 SPI
//! `fsgetpath()` 把 (fsid, objid) 还原成路径。控制流对齐参考实现
//! `../../../Open_Ref/searchfs/main.m`：打包 name 到 searchparams1、请求返回
//! `ATTR_CMN_FSID | ATTR_CMN_OBJID`、EAGAIN 续搜、EBUSY 有界重试、Catalina+
//! 双卷（`/` 与 `/System/Volumes/Data`）。
//!
//! 非 macOS 目标编译成空 stub，保证 crate 在其他平台仍可 type-check。

/// 兜底引擎的搜索参数（比索引引擎简单：只做子串 + 文件/目录过滤）。
#[derive(Clone, Debug)]
pub struct FallbackOptions {
    pub query: String,
    pub dirs_only: bool,
    pub files_only: bool,
    pub limit: usize,
}

impl Default for FallbackOptions {
    fn default() -> Self {
        FallbackOptions {
            query: String::new(),
            dirs_only: false,
            files_only: false,
            limit: 2000,
        }
    }
}

/// 一条兜底命中。
#[derive(Clone, Debug)]
pub struct FallbackHit {
    pub path: String,
    pub is_dir: bool,
}

pub const DEFAULT_VOLUME: &str = "/";
pub const DATA_VOLUME: &str = "/System/Volumes/Data";

#[cfg(target_os = "macos")]
mod ffi {
    #![allow(non_camel_case_types)]
    // NB: 不用 `libc::u_int32_t`（新版 libc 已移除该 BSD 遗留 typedef），直接用 `u32`。
    use libc::{c_char, c_int, c_uint, c_void, timeval};

    pub const PATH_MAX: usize = 1024;
    pub const ATTR_BIT_MAP_COUNT: u16 = 5;

    pub const ATTR_CMN_NAME: u32 = 0x0000_0001;
    pub const ATTR_CMN_OBJID: u32 = 0x0000_0020;
    pub const ATTR_CMN_FSID: u32 = 0x0000_0040;

    pub const ATTR_VOL_INFO: u32 = 0x8000_0000;
    pub const ATTR_VOL_CAPABILITIES: u32 = 0x0002_0000;
    pub const VOL_CAPABILITIES_INTERFACES: usize = 1;
    pub const VOL_CAP_INT_SEARCHFS: u32 = 0x0000_0001;

    pub const SRCHFS_START: c_uint = 0x0000_0001;
    pub const SRCHFS_MATCHPARTIALNAMES: c_uint = 0x0000_0002;
    pub const SRCHFS_MATCHDIRS: c_uint = 0x0000_0004;
    pub const SRCHFS_MATCHFILES: c_uint = 0x0000_0008;

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

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct attrreference {
        pub attr_dataoffset: i32,
        pub attr_length: u32,
    }

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

    #[repr(C)]
    pub struct searchstate {
        pub reserved: [u8; 556],
    }

    impl Default for searchstate {
        fn default() -> Self {
            searchstate { reserved: [0u8; 556] }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct fsid_t {
        pub val: [i32; 2],
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct fsobj_id_t {
        pub fid_objno: u32,
        pub fid_generation: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct packed_result {
        pub size: u32,
        pub fs_id: fsid_t,
        pub obj_id: fsobj_id_t,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct vol_capabilities_attr {
        pub capabilities: [u32; 4],
        pub valid: [u32; 4],
    }

    extern "C" {
        pub fn searchfs(
            path: *const c_char,
            search_block: *mut fssearchblock,
            num_matches: *mut libc::c_ulong,
            script_code: c_uint,
            options: c_uint,
            state: *mut searchstate,
        ) -> c_int;

        pub fn fsgetpath(
            buf: *mut c_char,
            bufsize: libc::size_t,
            fsid: *const fsid_t,
            obj_id: u64,
        ) -> libc::ssize_t;

        pub fn getattrlist(
            path: *const c_char,
            attr_list: *mut attrlist,
            attr_buf: *mut c_void,
            attr_buf_size: libc::size_t,
            options: libc::c_ulong,
        ) -> c_int;
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::ffi::*;
    use super::{FallbackHit, FallbackOptions, DATA_VOLUME, DEFAULT_VOLUME};
    use libc::{c_ulong, timeval, EAGAIN, EBUSY};
    use std::ffi::CString;
    use std::mem;
    use std::path::Path;

    const MAX_MATCHES: usize = 256;
    const MAX_EBUSY_RETRIES: u32 = 5;

    #[repr(C)]
    struct PackedNameAttr {
        size: u32,
        ref_: attrreference,
        name: [u8; PATH_MAX],
    }

    #[repr(C)]
    struct PackedAttrRef {
        size: u32,
        ref_: attrreference,
    }

    /// 探测 `path` 是否是支持 searchfs 的已挂载卷。
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

    fn data_volume_available() -> bool {
        Path::new(DATA_VOLUME).exists() && volume_supports_searchfs(DATA_VOLUME)
    }

    fn passes_filters(is_dir: bool, opts: &FallbackOptions) -> bool {
        if opts.dirs_only && !is_dir {
            return false;
        }
        if opts.files_only && is_dir {
            return false;
        }
        true
    }

    fn errno() -> i32 {
        unsafe { *libc::__error() }
    }

    fn search_one_volume(volpath: &str, opts: &FallbackOptions, out: &mut Vec<FallbackHit>) {
        let c_vol = match CString::new(volpath) {
            Ok(v) => v,
            Err(_) => return,
        };
        let query_bytes = opts.query.as_bytes();
        if query_bytes.is_empty() || query_bytes.len() >= PATH_MAX {
            return;
        }

        let mut ebusy_count = 0u32;
        let mut result_buffer = [packed_result::default(); MAX_MATCHES];

        let mut info1: PackedNameAttr = unsafe { mem::zeroed() };
        info1.name[..query_bytes.len()].copy_from_slice(query_bytes);
        info1.name[query_bytes.len()] = 0;
        info1.ref_.attr_dataoffset = mem::size_of::<attrreference>() as i32;
        info1.ref_.attr_length = (query_bytes.len() + 1) as u32;
        info1.size = mem::size_of::<attrreference>() as u32 + info1.ref_.attr_length;

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

        let mut base_options: libc::c_uint = SRCHFS_START | SRCHFS_MATCHPARTIALNAMES;
        if !opts.dirs_only {
            base_options |= SRCHFS_MATCHFILES;
        }
        if !opts.files_only {
            base_options |= SRCHFS_MATCHDIRS;
        }

        // `catalog_changed` 重启标签：EBUSY 时整趟重来（有界）。
        'restart: loop {
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
            let mut this_options = base_options;

            loop {
                let mut matches: c_ulong = 0;
                let mut err = unsafe {
                    searchfs(c_vol.as_ptr(), &mut block, &mut matches, 0, this_options, &mut state)
                };
                if err == -1 {
                    err = errno();
                }

                if (err == 0 || err == EAGAIN) && matches > 0 {
                    unpack_results(&result_buffer, matches as usize, opts, out);
                    if opts.limit != 0 && out.len() >= opts.limit {
                        return;
                    }
                }

                if err == EBUSY && ebusy_count < MAX_EBUSY_RETRIES {
                    ebusy_count += 1;
                    continue 'restart; // catalog 变更，整趟重来
                }
                if err != 0 && err != EAGAIN {
                    return; // 真实失败（如无全盘访问权限的 EPERM）
                }
                this_options &= !SRCHFS_START;
                if err != EAGAIN {
                    return;
                }
            }
        }
    }

    fn unpack_results(
        buffer: &[packed_result],
        matches: usize,
        opts: &FallbackOptions,
        out: &mut Vec<FallbackHit>,
    ) {
        for r in buffer.iter().take(matches) {
            let obj_id: u64 =
                (r.obj_id.fid_objno as u64) | ((r.obj_id.fid_generation as u64) << 32);
            let mut path_buf = [0i8; PATH_MAX];
            let size = unsafe { fsgetpath(path_buf.as_mut_ptr(), PATH_MAX, &r.fs_id, obj_id) };
            if size < 0 {
                continue; // 已删除，静默跳过
            }
            let bytes = unsafe {
                std::slice::from_raw_parts(path_buf.as_ptr() as *const u8, size as usize)
            };
            let path = match std::str::from_utf8(bytes) {
                Ok(s) => s.to_string(),
                Err(_) => String::from_utf8_lossy(bytes).into_owned(),
            };
            let is_dir = std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false);
            if !passes_filters(is_dir, opts) {
                continue;
            }
            out.push(FallbackHit { path, is_dir });
            if opts.limit != 0 && out.len() >= opts.limit {
                return;
            }
        }
    }

    pub fn search(opts: &FallbackOptions) -> Vec<FallbackHit> {
        let mut out = Vec::new();
        if opts.query.is_empty() {
            return out;
        }
        search_one_volume(DEFAULT_VOLUME, opts, &mut out);
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

    /// 是否至少有一个默认卷支持 searchfs（用于 doctor / 引擎自检）。
    pub fn available() -> bool {
        volume_supports_searchfs(DEFAULT_VOLUME) || data_volume_available()
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::{FallbackHit, FallbackOptions};

    /// 非 macOS：searchfs 不存在，返回空。保证 crate 在 Linux/CI-lint 上可编译，
    /// 真实目标始终是 macos-latest。
    pub fn search(_opts: &FallbackOptions) -> Vec<FallbackHit> {
        Vec::new()
    }

    pub fn volume_supports_searchfs(_path: &str) -> bool {
        false
    }

    pub fn available() -> bool {
        false
    }
}

pub use imp::{available, search, volume_supports_searchfs};

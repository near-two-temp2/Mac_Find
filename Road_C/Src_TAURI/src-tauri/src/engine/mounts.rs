//! Mount-point classification for safe indexing.
//!
//! The index walker must **never** descend into network / FUSE mounts:
//!   - `/Volumes/Disk/h2-*` are rclone → Backblaze B2 mounts (macFUSE). Walking
//!     them is slow, can hang, and — worst — burns paid B2 Class-C API quota.
//!   - `~/Library/CloudStorage/{GoogleDrive,OneDrive}-*` are FileProvider mounts
//!     that trigger network round-trips on deep traversal.
//!   - NFS/SMB/AFP/WebDAV/sshfs shares are similarly remote.
//!
//! We take a belt-and-suspenders approach (see `../../SEARCH_TEST_BASELINE.md`):
//!   1. Query the kernel mount table (`getfsstat`) and prune any mount whose
//!      `f_fstypename` is not a known-local type (apfs/hfs) or that lacks the
//!      `MNT_LOCAL` flag. This is the authoritative, general check.
//!   2. Additionally prune a small set of hardcoded known-bad path prefixes,
//!      so we stay safe even if a mount's fstype somehow reads as local or the
//!      syscall is unavailable.
//!
//! Only the syscall part is macOS-specific; on other targets we fall back to
//! the hardcoded prefixes so the crate still builds and behaves sanely.

use std::path::Path;

/// Filesystem types we consider local and safe to index. Everything else
/// (macfuse, nfs, smbfs, afpfs, webdav, sshfs, …) is pruned.
const LOCAL_FSTYPES: &[&str] = &["apfs", "hfs"];

/// Known path prefixes that must never be walked, independent of what the
/// mount table reports. Values here are lowercased at compare time.
///
/// `~/Library/CloudStorage` is expanded against `$HOME` at runtime (see
/// `NetworkGuard::new`); the `/Volumes/Disk/h2` entries come straight from the
/// project's CLAUDE.md list of rclone→B2 mounts.
const HARDCODED_EXCLUDES: &[&str] = &[
    "/volumes/disk/h2-bu-01",
    "/volumes/disk/h2_bu_01_b2",
    "/volumes/disk/h2_open_rsh",
];

/// Precomputed set of directories the walker must not descend into.
///
/// Built once per index build; `should_prune` is then a cheap prefix test the
/// hot loop can call for every directory entry.
pub struct NetworkGuard {
    /// Non-local mount points discovered from the kernel mount table, plus the
    /// hardcoded excludes. Stored lowercased, without a trailing slash.
    excluded: Vec<String>,
}

impl NetworkGuard {
    /// Enumerate mounts and assemble the prune set for the current machine.
    pub fn new() -> Self {
        let mut excluded: Vec<String> = Vec::new();

        // 1) Non-local mounts straight from the kernel.
        for m in non_local_mounts() {
            push_norm(&mut excluded, &m);
        }

        // 2) Hardcoded known-bad prefixes.
        for e in HARDCODED_EXCLUDES {
            push_norm(&mut excluded, e);
        }

        // 3) The whole CloudStorage FileProvider tree under $HOME.
        if let Some(home) = std::env::var_os("HOME") {
            let cs = Path::new(&home).join("Library").join("CloudStorage");
            if let Some(s) = cs.to_str() {
                push_norm(&mut excluded, s);
            }
        }

        // Dedup so the per-entry check stays short.
        excluded.sort();
        excluded.dedup();

        NetworkGuard { excluded }
    }

    /// True when `path` is on (or under) an excluded network/FUSE mount and the
    /// walker should not descend into it.
    ///
    /// Matches on a normalized, lowercased path with segment-boundary awareness
    /// so `/volumes/disk/h2-bu-01x` is *not* falsely pruned by the prefix
    /// `/volumes/disk/h2-bu-01`.
    pub fn should_prune(&self, path: &Path) -> bool {
        let p = match path.to_str() {
            Some(s) => s.trim_end_matches('/').to_ascii_lowercase(),
            None => return false,
        };
        for ex in &self.excluded {
            if p == *ex {
                return true;
            }
            // `p` is under `ex` iff it starts with "ex/".
            if p.len() > ex.len()
                && p.as_bytes()[ex.len()] == b'/'
                && p.starts_with(ex.as_str())
            {
                return true;
            }
        }
        false
    }

    /// For diagnostics/tests: how many mount points are being excluded.
    pub fn excluded_len(&self) -> usize {
        self.excluded.len()
    }
}

impl Default for NetworkGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize (strip trailing slash, lowercase) and push a path prefix.
fn push_norm(v: &mut Vec<String>, s: &str) {
    let n = s.trim_end_matches('/').to_ascii_lowercase();
    if !n.is_empty() {
        v.push(n);
    }
}

/// Return the mount points of all mounted filesystems that are **not** local
/// (i.e. network / FUSE / removable-remote). On non-macOS this is empty and we
/// rely solely on the hardcoded excludes.
#[cfg(target_os = "macos")]
fn non_local_mounts() -> Vec<String> {
    imp::non_local_mounts()
}

#[cfg(not(target_os = "macos"))]
fn non_local_mounts() -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "macos")]
mod imp {
    use super::LOCAL_FSTYPES;
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_int, c_uint};

    // <sys/mount.h> constants and struct. `statfs` is the 64-bit-inode variant
    // on modern macOS (the historical `statfs64` alias resolves to this).
    const MFSTYPENAMELEN: usize = 16;
    const MAXPATHLEN: usize = 1024;
    const MNT_LOCAL: u32 = 0x0000_1000; // filesystem is stored locally
    const MNT_NOWAIT: c_int = 2; // don't block on remote fs when listing

    #[repr(C)]
    struct Statfs {
        f_bsize: u32,
        f_iosize: i32,
        f_blocks: u64,
        f_bfree: u64,
        f_bavail: u64,
        f_files: u64,
        f_ffree: u64,
        f_fsid: [i32; 2],
        f_owner: u32,
        f_type: u32,
        f_flags: u32,
        f_fssubtype: u32,
        f_fstypename: [c_char; MFSTYPENAMELEN],
        f_mntonname: [c_char; MAXPATHLEN],
        f_mntfromname: [c_char; MAXPATHLEN],
        f_flags_ext: u32,
        f_reserved: [u32; 7],
    }

    extern "C" {
        // getfsstat(struct statfs *buf, int bufsize, int flags)
        // The unsuffixed symbol is the INODE64 variant on x86_64 & arm64 macOS.
        fn getfsstat(buf: *mut Statfs, bufsize: c_int, flags: c_int) -> c_int;
    }

    fn c_to_string(raw: &[c_char]) -> String {
        // SAFETY: the kernel NUL-terminates these fixed-size fields.
        let cstr = unsafe { CStr::from_ptr(raw.as_ptr()) };
        cstr.to_string_lossy().into_owned()
    }

    pub fn non_local_mounts() -> Vec<String> {
        // First call with a null buffer to learn how many mounts exist.
        let count = unsafe { getfsstat(std::ptr::null_mut(), 0, MNT_NOWAIT) };
        if count <= 0 {
            return Vec::new();
        }
        // Over-allocate a little in case mounts appear between the two calls.
        let cap = (count as usize) + 8;
        let mut bufs: Vec<Statfs> = Vec::with_capacity(cap);
        let byte_size = (cap * std::mem::size_of::<Statfs>()) as c_int;
        let n = unsafe { getfsstat(bufs.as_mut_ptr(), byte_size, MNT_NOWAIT) };
        if n <= 0 {
            return Vec::new();
        }
        // SAFETY: the kernel wrote `n` fully-initialized Statfs records.
        unsafe { bufs.set_len(n as usize) };

        let mut out = Vec::new();
        for fs in &bufs {
            let fstype = c_to_string(&fs.f_fstypename).to_ascii_lowercase();
            let is_local_flag = (fs.f_flags & MNT_LOCAL) != 0;
            let is_local_type = LOCAL_FSTYPES.iter().any(|t| *t == fstype);
            // Prune anything that is not clearly a local apfs/hfs volume. Being
            // conservative here is the safe direction: at worst we skip a
            // local exotic filesystem; we never accidentally walk a remote one.
            if !(is_local_flag && is_local_type) {
                out.push(c_to_string(&fs.f_mntonname));
            }
        }
        out
    }

    // Silence unused warnings for constants kept for documentation parity.
    #[allow(dead_code)]
    const _MNT_NOWAIT_DOC: c_int = MNT_NOWAIT;
    #[allow(dead_code)]
    const _MFS_DOC: c_uint = MFSTYPENAMELEN as c_uint;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn prunes_known_b2_mounts() {
        let g = NetworkGuard::new();
        assert!(g.should_prune(&PathBuf::from("/Volumes/Disk/h2-bu-01")));
        assert!(g.should_prune(&PathBuf::from("/Volumes/Disk/h2_bu_01_b2/sub/dir")));
        assert!(g.should_prune(&PathBuf::from("/Volumes/Disk/h2_open_rsh")));
    }

    #[test]
    fn does_not_prune_local_siblings() {
        let g = NetworkGuard::new();
        // A sibling whose name merely starts with an excluded prefix must NOT
        // be pruned — segment boundary matters.
        assert!(!g.should_prune(&PathBuf::from("/Volumes/Disk/h2-bu-01x")));
        assert!(!g.should_prune(&PathBuf::from("/Volumes/Disk/other")));
        assert!(!g.should_prune(&PathBuf::from("/Users/oracle/temp_test")));
    }

    #[test]
    fn prunes_cloudstorage_when_home_set() {
        // Set HOME so the CloudStorage rule has a concrete prefix to match.
        std::env::set_var("HOME", "/Users/tester");
        let g = NetworkGuard::new();
        assert!(g.should_prune(&PathBuf::from(
            "/Users/tester/Library/CloudStorage/GoogleDrive-x@y.com/My Drive"
        )));
        assert!(!g.should_prune(&PathBuf::from("/Users/tester/Documents")));
    }

    #[test]
    fn case_insensitive_match() {
        let g = NetworkGuard::new();
        assert!(g.should_prune(&PathBuf::from("/VOLUMES/DISK/H2-BU-01/x")));
    }
}

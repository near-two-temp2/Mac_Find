//! 建索引避网络盘 —— 只索引本地卷，跳过所有 FUSE / 网络 / 云同步挂载。
//!
//! 为什么关键（见 ../../../SEARCH_TEST_BASELINE.md §索引构建硬性要求）：
//! 本机有 rclone→Backblaze B2 的 macFUSE 挂载。深度遍历它们会：极慢、可能
//! 挂死、并**触发 B2 的 API 配额计费**。所以建索引时必须把网络盘整段 prune 掉。
//!
//! 三重防线（任一命中即跳过一个目录子树）：
//!   1. **fstype 白名单**：用 `getmntinfo`/`statfs` 读挂载点 `f_fstypename`，
//!      只保留本地文件系统（`apfs`/`hfs`/`hfs+`）。非白名单（macfuse/nfs/
//!      smbfs/afpfs/webdav/…）一律视为网络盘。
//!   2. **不跨设备**：遍历时比对 `st_dev`，遇到与根不同的设备号（挂载边界）
//!      就 prune —— 子挂载（如 `/Volumes/Disk/h2-*`）自然被切掉。
//!   3. **显式黑名单兜底**：对已知的 rclone→B2 挂载点与 CloudStorage 目录
//!      直接排除，双保险。
//!
//! 非 macOS 目标退化为「只做显式黑名单 + 不跨设备」，保证可编译（CI lint）。

use std::path::Path;

/// 已知必须跳过的挂载点 / 目录前缀（来自项目 CLAUDE.md 与测试基线）。
///
/// 这些是 rclone→Backblaze B2 的 macFUSE 挂载与云同步 FileProvider 目录，
/// 深度遍历会烧 B2 配额或极慢。`~` 由调用方在运行时展开为 `$HOME`。
const EXPLICIT_DENY_ABS: &[&str] = &[
    "/Volumes/Disk/h2-bu-01",
    "/Volumes/Disk/h2_bu_01_b2",
    "/Volumes/Disk/h2_open_rsh",
];

/// `$HOME` 下必须跳过的相对前缀（云同步 FileProvider）。
const EXPLICIT_DENY_HOME_REL: &[&str] = &[
    "Library/CloudStorage",
];

/// 判断一个路径是否落在显式黑名单里（网络 / 云盘），应整段 prune。
pub fn is_explicitly_denied(path: &Path) -> bool {
    let p = path.to_string_lossy();
    for deny in EXPLICIT_DENY_ABS {
        if p == *deny || p.starts_with(&format!("{deny}/")) {
            return true;
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = Path::new(&home);
        for rel in EXPLICIT_DENY_HOME_REL {
            let denied = home.join(rel);
            let d = denied.to_string_lossy();
            if p == d || p.starts_with(&format!("{d}/")) {
                return true;
            }
        }
    }
    false
}

/// macOS：本地文件系统类型白名单（`f_fstypename`）。只有这些才索引。
///
/// 网络 / FUSE 类型（`macfuse`/`nfs`/`smbfs`/`afpfs`/`cifs`/`webdav`/`osxfuse`/
/// FileProvider…）都不在白名单里 → 跳过。
#[cfg(target_os = "macos")]
const LOCAL_FSTYPES: &[&str] = &["apfs", "hfs", "hfs+", "msdos", "exfat"];

/// 读某路径所在卷的 `f_fstypename`（macOS，用 `statfs`）。
///
/// 返回 `None` 表示 `statfs` 失败（路径不存在 / 无权限）；调用方应保守处理。
#[cfg(target_os = "macos")]
pub fn fstype_of(path: &Path) -> Option<String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut sfs: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(c_path.as_ptr(), &mut sfs) };
    if rc != 0 {
        return None;
    }
    // f_fstypename 是 [c_char; MFSTYPENAMELEN]，以 NUL 结尾。
    let raw = &sfs.f_fstypename;
    let bytes: Vec<u8> = raw
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    Some(String::from_utf8_lossy(&bytes).to_ascii_lowercase())
}

#[cfg(not(target_os = "macos"))]
pub fn fstype_of(_path: &Path) -> Option<String> {
    None
}

/// 该路径所在卷是否为「本地、可安全索引」的卷。
///
/// 判定：显式黑名单命中 → false；否则看 fstype 是否在本地白名单里。
/// fstype 读不到时（非 macOS / statfs 失败）保守返回 true，交由「不跨设备」
/// 与「显式黑名单」两道防线兜底（避免误伤本地卷导致漏索引）。
pub fn is_local_indexable(path: &Path) -> bool {
    if is_explicitly_denied(path) {
        return false;
    }
    match fstype_of(path) {
        #[cfg(target_os = "macos")]
        Some(fs) => LOCAL_FSTYPES.iter().any(|t| *t == fs),
        #[cfg(not(target_os = "macos"))]
        Some(_) => true,
        None => true,
    }
}

/// 取路径所在设备号 `st_dev`（用于遍历时的「不跨设备」判定）。
///
/// 遍历中若某目录的 `st_dev` 与根不同，说明踩到了一个（子）挂载边界，
/// prune 掉即可 —— 天然切掉 `/Volumes/Disk/h2-*` 这类挂在本地卷下的网络子盘。
pub fn dev_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).ok().map(|m| m.dev() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn explicit_deny_matches_known_mounts() {
        assert!(is_explicitly_denied(Path::new("/Volumes/Disk/h2-bu-01")));
        assert!(is_explicitly_denied(Path::new(
            "/Volumes/Disk/h2_bu_01_b2/some/deep/file"
        )));
        // 前缀不完整不应误伤（h2-bu-01x 不是 h2-bu-01 的子路径）。
        assert!(!is_explicitly_denied(Path::new("/Volumes/Disk/h2-bu-01x")));
        // 普通本地路径不拒。
        assert!(!is_explicitly_denied(Path::new("/Users/oracle/temp_test")));
    }

    #[test]
    fn cloudstorage_denied_under_home() {
        if let Some(home) = std::env::var_os("HOME") {
            let p: PathBuf = Path::new(&home)
                .join("Library/CloudStorage/GoogleDrive-x/Foo");
            assert!(is_explicitly_denied(&p));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn root_volume_is_local() {
        // `/` 在开发机与 CI runner 上都是 apfs。
        assert!(is_local_indexable(Path::new("/")));
        let fs = fstype_of(Path::new("/")).unwrap();
        assert!(
            LOCAL_FSTYPES.iter().any(|t| *t == fs),
            "root fstype should be local, got {fs}"
        );
    }
}

//! 「在 Finder 中显示」/「打开」—— GUI 结果列表的操作。
//!
//! 用 macOS 内置的 `open` 命令实现，不引额外依赖：
//!   - `open -R <path>`  在 Finder 中定位并选中该文件；
//!   - `open <path>`     用默认应用打开。
//!
//! 非 macOS 下为 no-op（返回 Ok），保证跨平台可编译。

use std::io;
use std::path::Path;

/// 在 Finder 中显示（选中）给定路径。
pub fn reveal_in_finder(path: &Path) -> io::Result<()> {
    run_open(&["-R".as_ref(), path.as_os_str()])
}

/// 用默认应用打开给定路径。
pub fn open_path(path: &Path) -> io::Result<()> {
    run_open(&[path.as_os_str()])
}

#[cfg(target_os = "macos")]
fn run_open(args: &[&std::ffi::OsStr]) -> io::Result<()> {
    use std::process::Command;
    let status = Command::new("/usr/bin/open").args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("`open` exited with {status}"),
        ))
    }
}

#[cfg(not(target_os = "macos"))]
fn run_open(_args: &[&std::ffi::OsStr]) -> io::Result<()> {
    // 非 macOS：无 Finder，静默成功。
    Ok(())
}

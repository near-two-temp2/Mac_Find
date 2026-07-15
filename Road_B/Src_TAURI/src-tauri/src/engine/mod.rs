//! 搜索引擎：自建二进制索引 + 两阶段 fzf 搜索。
//!
//! 该模块与 Road_B/Src_RUST（egui 实现）共用同一套二进制索引契约，
//! 只是这里被 Tauri 后端（`#[tauri::command]`）与 `haifind-tauri-search` CLI 复用。
//!
//! 子模块：
//!   - [`bitmask`] —— 64-bit 字母 bitmask 编码 + 词边界位图（Phase 1 预过滤基石）。
//!   - [`index`]   —— 建索引（并行数组落盘）与 mmap 零拷贝读取。
//!   - [`search`]  —— 两阶段搜索（rayon 并行预过滤 + fzf 评分）。

pub mod bitmask;
pub mod index;
pub mod search;

pub use index::{IndexReader, IndexStats, IndexWriter};
pub use search::{search, KindFilter, Match, SearchOptions};

use std::path::PathBuf;

/// 默认索引文件位置：`~/Library/Caches/com.haifind.b-tauri/index.idx`
///
/// 与 Cling 的 `~/Library/Caches/com.lowtechguys.Cling/*.idx` 思路一致：
/// 索引落在用户缓存目录，磁盘空间紧张时可安全删除。
pub fn default_index_path() -> PathBuf {
    let base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("com.haifind.b-tauri").join("index.idx")
}

//! Road_B (Rust) — 自建二进制索引 + fzf 模糊搜索引擎
//!
//! 架构参考 Cling（见 ../../../open-source-analysis.md §3）：
//!   - 建索引：遍历文件系统 → 写 mmap 友好的二进制索引（并行数组）。
//!   - 索引字段：小写路径字节、64-bit 字母 bitmask、basename bitmask、
//!     词边界位图、扩展名 ID。
//!   - 两阶段搜索：
//!       Phase 1 —— rayon 并行 bitmask + 扩展名 O(n) 预过滤；
//!       Phase 2 —— 对存活候选做 fzf 评分排序。
//!
//! 该库被 GUI（`haifind-gui`）与两个 CLI（`haifind-index` / `haifind-search`）复用。

pub mod bitmask;
pub mod index;
pub mod search;

pub use index::{IndexReader, IndexWriter, IndexStats};
pub use search::{search, Match, SearchOptions};

use std::path::PathBuf;

/// 默认索引文件位置：`~/Library/Caches/com.haifind.b-rust/index.idx`
///
/// 与 Cling 的 `~/Library/Caches/com.lowtechguys.Cling/*.idx` 一致的思路：
/// 索引落在用户缓存目录，磁盘空间紧张时可安全删除。
pub fn default_index_path() -> PathBuf {
    let base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("com.haifind.b-rust").join("index.idx")
}

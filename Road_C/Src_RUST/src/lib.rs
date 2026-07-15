//! Road_C (Rust) — macOS 极速文件搜索「混合引擎」库。
//!
//! 完整版混合架构（见 ../../../open-source-analysis.md §5.4）：
//!   - **主路径**：自建 mmap 二进制索引 + rayon 并行 bitmask 预过滤 + fzf 评分；
//!   - **兜底路径**：索引缺失/损坏/为空时，降级到 libc FFI 调 `searchfs()` 实时扫描。
//!
//! 该库被 GUI（`haifind-c-gui`）与 CLI（`haifind-c-cli`）复用，二者共享
//! [`HybridEngine`] 与结果类型。

pub mod bitmask;
pub mod engine;
pub mod fuzzy;
pub mod index;
pub mod reveal;
pub mod searchfs;

pub use engine::{
    build_index, default_roots, Backend, HybridEngine, Match, SearchOptions, SearchResult,
};
pub use index::{default_index_path, IndexReader, IndexStats, IndexWriter};

//! 混合搜索引擎 —— Road_C 的核心编排层。
//!
//! 决策逻辑（对齐 open-source-analysis.md §5.4 推荐架构）：
//!
//! ```text
//!            ┌───────────────┐
//!  query ──▶ │  HybridEngine │
//!            └──────┬────────┘
//!                   │ 索引可用（mmap 打开成功且非空）？
//!         ┌─────────┴──────────┐
//!        是                    否 / 损坏 / 空
//!         ▼                     ▼
//!  ┌─────────────┐      ┌──────────────────┐
//!  │ 索引引擎(主) │      │ searchfs()兜底(备) │
//!  │ Phase1 并行  │      │ 实时 catalog 扫描  │
//!  │ bitmask 预过滤│      │ 100% 准确、无需索引│
//!  │ Phase2 fzf   │      └──────────────────┘
//!  └─────────────┘
//! ```
//!
//! GUI 与 CLI 都通过 [`HybridEngine`] 访问，二者共享同一套结果类型。

use crate::bitmask::mask_of;
use crate::fuzzy;
use crate::index::{default_index_path, IndexReader, IndexStats, IndexWriter};
use crate::searchfs::{self, FallbackOptions};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 一条最终结果，供 GUI 列表 / CLI 打印。
#[derive(Clone, Debug)]
pub struct Match {
    pub path: String,
    pub is_dir: bool,
    /// fzf 分数；兜底路径统一给 0（未评分）。
    pub score: i32,
}

/// 用户搜索参数。
#[derive(Clone, Debug)]
pub struct SearchOptions {
    pub query: String,
    pub dirs_only: bool,
    pub files_only: bool,
    /// 结果上限（0 = 不限）。
    pub limit: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions {
            query: String::new(),
            dirs_only: false,
            files_only: false,
            limit: 1000,
        }
    }
}

/// 本次查询实际走了哪条路径 —— GUI 用它给用户显示「索引 / 实时兜底」状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// 走了自建索引（主路径）。
    Index,
    /// 索引缺失/损坏/空 → searchfs() 实时兜底。
    SearchfsFallback,
    /// 兜底也不可用（非 macOS，或无 searchfs 支持的卷）。
    Unavailable,
}

/// 一次查询的结果 + 用了哪个后端。
pub struct SearchResult {
    pub matches: Vec<Match>,
    pub backend: Backend,
}

/// 混合引擎：持有（可选的）已加载索引，按需降级到 searchfs。
pub struct HybridEngine {
    index: Option<Arc<IndexReader>>,
    index_path: PathBuf,
}

impl HybridEngine {
    /// 用默认索引路径构造，尝试 mmap 打开现有索引（失败即视为无索引）。
    pub fn new() -> Self {
        Self::with_index_path(default_index_path())
    }

    /// 用指定索引路径构造。
    pub fn with_index_path(index_path: PathBuf) -> Self {
        let index = Self::try_open(&index_path);
        HybridEngine { index, index_path }
    }

    fn try_open(path: &Path) -> Option<Arc<IndexReader>> {
        match IndexReader::open(path) {
            Ok(r) if !r.is_empty() => Some(Arc::new(r)),
            // 打不开（不存在）或损坏或空 → 无索引，交给兜底。
            _ => None,
        }
    }

    /// 索引当前是否已加载可用。
    pub fn has_index(&self) -> bool {
        self.index.is_some()
    }

    /// 已加载索引的条目数（无索引时为 0）。
    pub fn index_len(&self) -> usize {
        self.index.as_ref().map(|r| r.len()).unwrap_or(0)
    }

    /// 索引文件路径。
    pub fn index_path(&self) -> &Path {
        &self.index_path
    }

    /// 重新尝试打开索引（建索引之后调用，热切回主路径）。
    pub fn reload_index(&mut self) {
        self.index = Self::try_open(&self.index_path);
    }

    /// 执行一次搜索，自动选择后端。
    pub fn search(&self, opts: &SearchOptions) -> SearchResult {
        if opts.query.is_empty() {
            return SearchResult {
                matches: Vec::new(),
                backend: if self.has_index() {
                    Backend::Index
                } else {
                    Backend::SearchfsFallback
                },
            };
        }

        // 主路径：自建索引。
        if let Some(reader) = &self.index {
            let matches = search_index(reader, opts);
            return SearchResult {
                matches,
                backend: Backend::Index,
            };
        }

        // 降级路径：searchfs() 实时兜底。
        if searchfs::available() {
            let fb = FallbackOptions {
                query: opts.query.clone(),
                dirs_only: opts.dirs_only,
                files_only: opts.files_only,
                limit: opts.limit,
            };
            let hits = searchfs::search(&fb);
            let matches = hits
                .into_iter()
                .map(|h| Match {
                    path: h.path,
                    is_dir: h.is_dir,
                    score: 0,
                })
                .collect();
            return SearchResult {
                matches,
                backend: Backend::SearchfsFallback,
            };
        }

        SearchResult {
            matches: Vec::new(),
            backend: Backend::Unavailable,
        }
    }
}

impl Default for HybridEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// 索引引擎两阶段搜索：
///   Phase 1 —— rayon 并行 bitmask 预过滤（O(n)，一条 AND 排除绝大多数）；
///   Phase 2 —— 对存活候选做 fzf 评分；
/// 最后按分数降序取前 `limit` 条。
fn search_index(reader: &IndexReader, opts: &SearchOptions) -> Vec<Match> {
    let query_lower: Vec<u8> = opts.query.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let query_mask = mask_of(&query_lower);
    let n = reader.len();

    // Phase 1 + Phase 2 融合并行：每个 entry 先 bitmask 预过滤，存活则 fzf 评分。
    // 优先用 basename 评分（更贴合「文件名搜索」直觉），basename 不中再退回全路径。
    let mut scored: Vec<Match> = (0..n)
        .into_par_iter()
        .filter_map(|i| {
            let e = reader.entry(i);

            // 文件/目录过滤。
            if opts.dirs_only && !e.is_dir {
                return None;
            }
            if opts.files_only && e.is_dir {
                return None;
            }

            // Phase 1：bitmask 预过滤（先看 basename mask，再看全路径 mask）。
            let bn_ok = crate::bitmask::could_contain(e.bn_mask, query_mask);
            let path_ok = crate::bitmask::could_contain(e.mask, query_mask);
            if !bn_ok && !path_ok {
                return None;
            }

            // Phase 2：fzf 评分。basename 命中额外加权（更相关）。
            let basename = e.basename();
            let mut best: Option<i32> = None;
            if bn_ok {
                if let Some((s, _, _)) = fuzzy::score(basename, &query_lower, e.boundaries) {
                    best = Some(s + BASENAME_BONUS);
                }
            }
            if path_ok {
                if let Some((s, _, _)) = fuzzy::score(e.path, &query_lower, e.boundaries) {
                    best = Some(best.map_or(s, |b| b.max(s)));
                }
            }
            let score = best?;

            let path = match std::str::from_utf8(e.path) {
                Ok(s) => s.to_string(),
                Err(_) => String::from_utf8_lossy(e.path).into_owned(),
            };
            Some(Match {
                path,
                is_dir: e.is_dir,
                score,
            })
        })
        .collect();

    // 按分数降序，同分按路径短优先（更「贴近根」的结果更有用）。
    scored.sort_unstable_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.path.len().cmp(&b.path.len()))
    });
    if opts.limit != 0 && scored.len() > opts.limit {
        scored.truncate(opts.limit);
    }
    scored
}

/// basename 命中的加权分（让「文件名里出现查询」的结果排在纯路径命中之前）。
const BASENAME_BONUS: i32 = 24;

/// 建索引：遍历给定根目录（默认 `$HOME`），写入索引文件。
///
/// 建索引不走 searchfs（那是查询兜底），而是用可移植的 `walkdir` 全量遍历，
/// 这样 CLI 冒烟测试在任何目录都能跑通。返回统计。
pub fn build_index(roots: &[PathBuf], index_path: &Path, follow_links: bool) -> std::io::Result<IndexStats> {
    let mut writer = IndexWriter::new();
    for root in roots {
        for entry in walkdir::WalkDir::new(root)
            .follow_links(follow_links)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if let Some(path) = entry.path().to_str() {
                let is_dir = entry.file_type().is_dir();
                writer.add(path, is_dir);
            }
        }
    }
    writer.finish(index_path)
}

/// 默认建索引根目录：用户主目录（磁盘紧张/无权限时也能安全遍历）。
pub fn default_roots() -> Vec<PathBuf> {
    match dirs::home_dir() {
        Some(h) => vec![h],
        None => vec![PathBuf::from(".")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_search_index() {
        let dir = std::env::temp_dir().join(format!("haifind-c-eng-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("hello_world.txt"), b"x").unwrap();
        std::fs::write(dir.join("sub").join("readme.md"), b"y").unwrap();

        let idx = dir.join("index.idx");
        let stats = build_index(&[dir.clone()], &idx, false).unwrap();
        assert!(stats.entries >= 3); // dir + sub + 2 files (+ dir itself)

        let engine = HybridEngine::with_index_path(idx);
        assert!(engine.has_index());

        let res = engine.search(&SearchOptions {
            query: "hello".into(),
            limit: 10,
            ..Default::default()
        });
        assert_eq!(res.backend, Backend::Index);
        assert!(res.matches.iter().any(|m| m.path.contains("hello_world.txt")));

        // 不存在的 query 无结果。
        let res2 = engine.search(&SearchOptions {
            query: "zzzznope".into(),
            ..Default::default()
        });
        assert!(res2.matches.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_index_uses_fallback_backend() {
        // 指向不存在的索引 → has_index 为 false。
        let engine = HybridEngine::with_index_path(
            std::env::temp_dir().join("haifind-c-nonexistent-xyz.idx"),
        );
        assert!(!engine.has_index());
        // 空 query 时后端标记为兜底（实际搜索时若无 searchfs 则 Unavailable）。
        let res = engine.search(&SearchOptions {
            query: String::new(),
            ..Default::default()
        });
        assert_eq!(res.backend, Backend::SearchfsFallback);
    }
}

//! 两阶段搜索：
//!   Phase 1 —— rayon 并行 bitmask + 扩展名 O(n) 预过滤，产出存活候选下标；
//!   Phase 2 —— 对存活候选做 fzf 评分（多锚点 + 连续/边界奖励 + 间隙惩罚），
//!             按分数降序取前 N 条。
//!
//! fzf 评分方案对齐 Cling（见 open-source-analysis.md §3.4）：
//! ```text
//! 字符匹配   +16
//! 连续匹配   +4
//! 首字符加成 ×2（命中 basename 首字符）
//! 词边界奖励 +9
//! 间隙开始   -3
//! 间隙延续   -1
//! ```

use crate::bitmask;
use crate::index::IndexReader;
use rayon::prelude::*;

/// 搜索选项。
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// 最多返回多少条结果。
    pub limit: usize,
    /// 仅目录 / 仅文件 / 全部。
    pub kind: KindFilter,
    /// 若查询形如 `foo.rs`，是否按扩展名 ID 快速过滤（Phase 1）。
    pub use_ext_filter: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 200,
            kind: KindFilter::All,
            use_ext_filter: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindFilter {
    All,
    FilesOnly,
    DirsOnly,
}

/// 一条搜索结果。
#[derive(Debug, Clone)]
pub struct Match {
    /// entry 下标。
    pub index: usize,
    /// fzf 分数（越高越好）。
    pub score: i32,
    /// 原始（小写）路径。
    pub path: String,
    /// 是否目录。
    pub is_dir: bool,
    /// 匹配窗口 [start, end)（在小写路径字节中），供高亮。
    pub match_start: usize,
    pub match_end: usize,
}

// 评分常量。
const SCORE_MATCH: i32 = 16;
const SCORE_CONSECUTIVE: i32 = 4;
const SCORE_BOUNDARY: i32 = 9;
const PENALTY_GAP_START: i32 = -3;
const PENALTY_GAP_EXTEND: i32 = -1;
const MAX_ANCHORS: usize = 32;

/// 顶层搜索入口：给定 mmap 索引与查询串，返回排序后的结果。
pub fn search(reader: &IndexReader, query: &str, opts: &SearchOptions) -> Vec<Match> {
    let n = reader.entry_count();
    if n == 0 {
        return Vec::new();
    }

    // 查询预处理：小写、拆分空格分隔的多 token（AND 语义，逐 token bitmask）。
    let query_lower: Vec<u8> = query
        .trim()
        .bytes()
        .map(|b| b.to_ascii_lowercase())
        .collect();

    // 空查询：直接按目录/文件过滤返回前 limit 条（默认视图）。
    if query_lower.is_empty() {
        return default_view(reader, opts);
    }

    let query_mask = bitmask::mask_of(&query_lower);

    // 若查询含扩展名（如 "x.rs"），预算目标 ext_id 以便 Phase 1 快速过滤。
    let target_ext = if opts.use_ext_filter {
        query_ext_id(&query_lower)
    } else {
        0
    };

    let masks = reader.masks();
    let bounds = reader.bounds();

    // ── Phase 1：rayon 并行预过滤 → 存活下标 ──
    let survivors: Vec<usize> = (0..n)
        .into_par_iter()
        .filter(|&i| {
            // 1) bitmask 预检
            if !bitmask::could_contain(masks[i], query_mask) {
                return false;
            }
            let meta = reader.meta(i);
            // 2) 目录/文件过滤
            match opts.kind {
                KindFilter::FilesOnly if meta.is_dir != 0 => return false,
                KindFilter::DirsOnly if meta.is_dir == 0 => return false,
                _ => {}
            }
            // 3) 扩展名快速过滤（仅当查询明确带扩展名时）
            if target_ext != 0 && meta.ext_id != target_ext {
                return false;
            }
            true
        })
        .collect();

    // ── Phase 2：对存活候选并行 fzf 评分 ──
    let mut scored: Vec<Match> = survivors
        .par_iter()
        .filter_map(|&i| {
            let text = reader.path_bytes(i);
            let meta = reader.meta(i);
            // 优先在 basename 上匹配（命中率与相关性更高），失败再退化到全路径。
            let bn = &text[meta.bn_start as usize..];
            let bn_off = meta.bn_start as usize;
            if let Some((mut score, s, e)) =
                fuzzy_score(&query_lower, bn, bounds[i], bn_off)
            {
                // basename 命中额外加成，让 "main.rs" 排在含 main 的深层路径之前。
                score += 12;
                return Some(make_match(reader, i, score, s, e, text, meta.is_dir != 0));
            }
            if let Some((score, s, e)) = fuzzy_score(&query_lower, text, bounds[i], 0) {
                return Some(make_match(reader, i, score, s, e, text, meta.is_dir != 0));
            }
            None
        })
        .collect();

    // 分数降序；同分时较短路径优先（更「精确」）。
    scored.sort_unstable_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.path.len().cmp(&b.path.len()))
    });
    scored.truncate(opts.limit);
    scored
}

fn make_match(
    _reader: &IndexReader,
    index: usize,
    score: i32,
    match_start: usize,
    match_end: usize,
    text: &[u8],
    is_dir: bool,
) -> Match {
    Match {
        index,
        score,
        path: String::from_utf8_lossy(text).into_owned(),
        is_dir,
        match_start,
        match_end,
    }
}

/// 默认视图：无查询时返回前 limit 条（受 kind 过滤）。
fn default_view(reader: &IndexReader, opts: &SearchOptions) -> Vec<Match> {
    let mut out = Vec::with_capacity(opts.limit);
    for i in 0..reader.entry_count() {
        let meta = reader.meta(i);
        match opts.kind {
            KindFilter::FilesOnly if meta.is_dir != 0 => continue,
            KindFilter::DirsOnly if meta.is_dir == 0 => continue,
            _ => {}
        }
        let text = reader.path_bytes(i);
        out.push(Match {
            index: i,
            score: 0,
            path: String::from_utf8_lossy(text).into_owned(),
            is_dir: meta.is_dir != 0,
            match_start: 0,
            match_end: 0,
        });
        if out.len() >= opts.limit {
            break;
        }
    }
    out
}

/// 从查询串推断扩展名 ID：查询含 '.' 且点后非空时才生效。
fn query_ext_id(query_lower: &[u8]) -> u32 {
    // 仅当整个查询看起来像 basename 片段（不含 '/'）时才用扩展名过滤，
    // 否则可能误伤（例如查询是路径片段 "a.b/c"）。
    if query_lower.contains(&b'/') {
        return 0;
    }
    crate::index::ext_id_of(query_lower)
}

/// fzf 评分核心：在 `text`（已小写）中模糊匹配 `pattern`（已小写）。
///
/// `boundaries` 是 `text` 的词边界位图，`bounds_off` 是 `text` 在原路径中的起始偏移
/// （因为边界位图是相对整条路径记录的，basename 视图需要加偏移对齐）。
///
/// 返回 `(score, start, end)`；若 pattern 无法作为子序列匹配则返回 None。
fn fuzzy_score(
    pattern: &[u8],
    text: &[u8],
    boundaries: u64,
    bounds_off: usize,
) -> Option<(i32, usize, usize)> {
    if pattern.is_empty() {
        return Some((0, 0, 0));
    }
    if pattern.len() > text.len() {
        return None;
    }

    let first = pattern[0];

    // 锚点枚举：first 在 text 中出现的位置（最多 MAX_ANCHORS 个）。
    let mut best: Option<(i32, usize, usize)> = None;
    let mut anchors = 0usize;
    let mut pos = 0usize;
    while pos < text.len() && anchors < MAX_ANCHORS {
        if text[pos] == first {
            anchors += 1;
            if let Some(cand) = score_from_anchor(pattern, text, boundaries, bounds_off, pos) {
                best = match best {
                    Some(b) if b.0 >= cand.0 => Some(b),
                    _ => Some(cand),
                };
            }
        }
        pos += 1;
    }
    best
}

/// 从锚点 `start` 开始正向贪婪匹配整个 pattern，逐字符累加分数。
fn score_from_anchor(
    pattern: &[u8],
    text: &[u8],
    boundaries: u64,
    bounds_off: usize,
    start: usize,
) -> Option<(i32, usize, usize)> {
    let mut score: i32 = 0;
    let mut pi = 0usize;
    let mut ti = start;
    let mut prev_match_idx: Option<usize> = None;
    let mut in_gap = false;
    let mut end = start;

    while pi < pattern.len() {
        // 在 text 中找到下一个 pattern[pi]。
        let mut found = None;
        let mut j = ti;
        while j < text.len() {
            if text[j] == pattern[pi] {
                found = Some(j);
                break;
            }
            j += 1;
        }
        let mi = found?; // 匹配不上 → 整体失败

        // 基础匹配分。
        score += SCORE_MATCH;

        // 词边界奖励（该 text 位置对应原路径的绝对位置在边界位图中置位）。
        let abs = bounds_off + mi;
        if abs < 64 && (boundaries & (1u64 << abs)) != 0 {
            score += SCORE_BOUNDARY;
        }
        // 首字符命中额外 ×2（即在 basename/text 起点）。
        if mi == 0 {
            score += SCORE_MATCH; // 相当于翻倍
        }

        // 连续 vs 间隙。
        match prev_match_idx {
            Some(p) if mi == p + 1 => {
                score += SCORE_CONSECUTIVE;
                in_gap = false;
            }
            Some(_) => {
                if in_gap {
                    score += PENALTY_GAP_EXTEND;
                } else {
                    score += PENALTY_GAP_START;
                    in_gap = true;
                }
            }
            None => {}
        }

        prev_match_idx = Some(mi);
        end = mi + 1;
        ti = mi + 1;
        pi += 1;
    }

    Some((score, start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexWriter;

    fn build(paths: &[(&str, bool)]) -> IndexReader {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let mut w = IndexWriter::new();
        for (p, d) in paths {
            w.add_path(p, *d);
        }
        // 每次调用唯一文件名：cargo test 默认并行，固定/仅 pid 的名字会让多个测试
        // 争用同一文件、读到截断内容而偶发失败。加原子计数器后缀彻底隔离。
        let tmp = std::env::temp_dir().join(format!(
            "haifind_search_test_{}_{}.idx",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        w.write_to(&tmp).unwrap();
        let r = IndexReader::open(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();
        r
    }

    #[test]
    fn finds_and_ranks() {
        let r = build(&[
            ("/Users/me/project/main.rs", false),
            ("/Users/me/project/README.md", false),
            ("/Users/me/project/src/main_helper.rs", false),
            ("/tmp/other/notes.txt", false),
        ]);
        let res = search(&r, "main.rs", &SearchOptions::default());
        assert!(!res.is_empty());
        // "main.rs" 精确 basename 应排第一
        assert!(res[0].path.ends_with("main.rs"));
        assert!(res[0].path.contains("project/main.rs"));
    }

    #[test]
    fn fuzzy_subsequence() {
        let r = build(&[("/a/bcd/hello_world.txt", false)]);
        // "hw" 作为子序列应命中 hello_world
        let res = search(&r, "hw", &SearchOptions::default());
        assert_eq!(res.len(), 1);
    }

    #[test]
    fn kind_filter() {
        let r = build(&[
            ("/a/foo", true),
            ("/a/foo.txt", false),
        ]);
        let mut opts = SearchOptions::default();
        opts.kind = KindFilter::DirsOnly;
        let res = search(&r, "foo", &opts);
        assert!(res.iter().all(|m| m.is_dir));
        assert!(!res.is_empty());
    }

    #[test]
    fn no_match_excluded_by_bitmask() {
        let r = build(&[("/a/abc.txt", false)]);
        // 'z' 不在路径中 → bitmask 直接排除
        let res = search(&r, "zzz", &SearchOptions::default());
        assert!(res.is_empty());
    }
}

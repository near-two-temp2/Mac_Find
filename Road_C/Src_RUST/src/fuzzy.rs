//! fzf 风格模糊评分 —— 混合引擎 Phase 2：对 bitmask 存活候选打分排序。
//!
//! 简化版 Cling 评分（见 open-source-analysis.md §3.4）：
//!   1. 锚点枚举：找 pattern 首字符在 text 中的所有出现位置；
//!   2. 正向贪婪匹配：从每个锚点顺序匹配 pattern 其余字符；
//!   3. 评分：字符匹配基础分 + 连续奖励 + 词边界奖励 - 间隙惩罚；
//!   4. 取所有锚点里的最高分。
//!
//! 输入 `text` / `pattern` 均为小写 ASCII/UTF-8 字节（调用方负责转小写）。
//! 返回 `None` 表示 pattern 无法作为 text 的子序列匹配。

/// 评分权重（对齐 open-source-analysis.md §3.4 的量级）。
const SCORE_MATCH: i32 = 16;
const SCORE_CONSECUTIVE: i32 = 4;
const SCORE_BOUNDARY: i32 = 8;
const FIRST_CHAR_MULT: i32 = 2;
const GAP_START: i32 = -3;
const GAP_EXTEND: i32 = -1;

/// 对 `text` 用 `pattern` 做一次 fzf 评分。
///
/// - `boundaries`：`text` 前 64 字节的词边界位图（见 [`crate::bitmask::word_boundaries`]）。
/// - 返回 `(score, start, end)`：分数越高越好，`start..end` 是命中窗口（供高亮用）。
pub fn score(text: &[u8], pattern: &[u8], boundaries: u64) -> Option<(i32, usize, usize)> {
    if pattern.is_empty() {
        return Some((0, 0, 0));
    }
    if pattern.len() > text.len() {
        return None;
    }

    let first = pattern[0];
    let mut best: Option<(i32, usize, usize)> = None;

    // 枚举首字符锚点。
    let mut anchor = 0usize;
    while let Some(rel) = memchr(first, &text[anchor..]) {
        let start = anchor + rel;
        if let Some((s, end)) = score_from(text, pattern, start, boundaries) {
            if best.map_or(true, |(bs, _, _)| s > bs) {
                best = Some((s, start, end));
            }
        }
        anchor = start + 1;
        if anchor >= text.len() {
            break;
        }
    }

    best
}

/// 从 `start`（已匹配 pattern[0]）开始贪婪匹配其余字符并累积分数。
fn score_from(
    text: &[u8],
    pattern: &[u8],
    start: usize,
    boundaries: u64,
) -> Option<(i32, usize)> {
    let mut score = 0i32;
    // `cursor` 是「从哪里开始找下一个字符」；`prev_mi` 是上一个匹配的位置。
    let mut cursor = start;
    let mut prev_mi: Option<usize> = None;

    for (pi, &pc) in pattern.iter().enumerate() {
        // 从 cursor 起找下一个匹配 pc 的位置。
        let mi = {
            let mut j = cursor;
            loop {
                if j >= text.len() {
                    return None; // 无法匹配整个 pattern → 该锚点作废
                }
                if text[j] == pc {
                    break j;
                }
                j += 1;
            }
        };

        let mut s = SCORE_MATCH;

        // 词边界奖励（该字节是否落在词起始）。
        if mi < 64 && (boundaries & (1u64 << mi)) != 0 {
            s += SCORE_BOUNDARY;
        }

        // 与上一个匹配相邻 → 连续奖励；否则按间隙长度惩罚。
        if let Some(pm) = prev_mi {
            let gap = mi - pm - 1; // pm 与 mi 之间未匹配的字符数
            if gap == 0 {
                s += SCORE_CONSECUTIVE;
            } else {
                s += GAP_START + GAP_EXTEND * (gap as i32 - 1);
            }
        }

        // 首字符加成（放在最后，让整段贡献翻倍）。
        if pi == 0 {
            s *= FIRST_CHAR_MULT;
        }

        score += s;
        prev_mi = Some(mi);
        cursor = mi + 1;
    }

    // end = 最后一个匹配位置 + 1（命中窗口右界）。
    Some((score, prev_mi.map_or(start, |m| m + 1)))
}

/// 极简单字节查找（避免引入 memchr crate，保持依赖精简）。
#[inline]
fn memchr(needle: u8, hay: &[u8]) -> Option<usize> {
    hay.iter().position(|&b| b == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitmask::word_boundaries;

    #[test]
    fn subsequence_matches() {
        let t = b"src/main.rs";
        let b = word_boundaries(t);
        assert!(score(t, b"main", b).is_some());
        assert!(score(t, b"srcmain", b).is_some());
        assert!(score(t, b"zzz", b).is_none());
    }

    #[test]
    fn boundary_beats_scattered() {
        let t = b"src/main.rs";
        let b = word_boundaries(t);
        // "main" 落在词边界，应比同长度散落匹配得分更高。
        let (main_score, _, _) = score(t, b"main", b).unwrap();
        let (scattered, _, _) = score(t, b"srn", b).unwrap();
        assert!(main_score > scattered);
    }
}

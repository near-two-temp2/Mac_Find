//! Minimal fzf-style fuzzy scorer, distilled from Cling's `fuzzyScoreBytes`
//! (see `open-source-analysis.md` §3.4). Operates on pre-lowercased bytes.
//!
//! Scoring model (mirrors the Cling table):
//!   +16  per matched character
//!   +4   consecutive-match bonus
//!   ×2   first-character-of-text bonus (applied to base)
//!   +9   separator boundary (`/ . - _` before the match char)
//!   +8   space boundary
//!   +7   camelCase boundary
//!   -3   gap start
//!   -1   gap continuation
//!
//! Returns `None` when the pattern is not a subsequence of the text.

const SCORE_MATCH: i32 = 16;
const SCORE_CONSECUTIVE: i32 = 4;
const BONUS_SEPARATOR: i32 = 9;
const BONUS_SPACE: i32 = 8;
const BONUS_CAMEL: i32 = 7;
const GAP_START: i32 = -3;
const GAP_CONT: i32 = -1;

#[inline]
fn is_separator(b: u8) -> bool {
    matches!(b, b'/' | b'.' | b'-' | b'_')
}

/// Score `pattern` (lowercase) against `text` (lowercase). `text_orig` is the
/// original-case text used only to detect camelCase boundaries.
pub fn score(pattern: &[u8], text: &[u8], text_orig: &[u8]) -> Option<i32> {
    if pattern.is_empty() {
        return Some(0);
    }
    if pattern.len() > text.len() {
        return None;
    }

    let mut total: i32 = 0;
    let mut pi = 0usize;
    let mut prev_match_idx: Option<usize> = None;

    let mut ti = 0usize;
    while ti < text.len() && pi < pattern.len() {
        if text[ti] == pattern[pi] {
            let mut char_score = SCORE_MATCH;

            // Boundary bonus based on the preceding character.
            if ti == 0 {
                char_score *= 2;
            } else {
                let prev = text[ti - 1];
                if is_separator(prev) {
                    char_score += BONUS_SEPARATOR;
                } else if prev == b' ' {
                    char_score += BONUS_SPACE;
                } else if text_orig.get(ti - 1).map_or(false, |c| c.is_ascii_lowercase())
                    && text_orig.get(ti).map_or(false, |c| c.is_ascii_uppercase())
                {
                    char_score += BONUS_CAMEL;
                }
            }

            // Consecutive / gap accounting.
            match prev_match_idx {
                Some(prev_idx) if prev_idx + 1 == ti => {
                    char_score += SCORE_CONSECUTIVE;
                }
                Some(prev_idx) => {
                    let gap = ti - prev_idx - 1;
                    char_score += GAP_START + GAP_CONT * (gap as i32 - 1).max(0);
                }
                None => {}
            }

            total += char_score;
            prev_match_idx = Some(ti);
            pi += 1;
        }
        ti += 1;
    }

    if pi == pattern.len() {
        Some(total)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(pat: &str, text: &str) -> Option<i32> {
        score(pat.as_bytes(), text.as_bytes(), text.as_bytes())
    }

    #[test]
    fn non_subsequence_is_none() {
        assert!(s("xyz", "hello").is_none());
    }

    #[test]
    fn exact_prefix_scores_high() {
        let a = s("read", "readme.md").unwrap();
        let b = s("read", "unrelated.md").unwrap();
        assert!(a > b, "prefix match should beat scattered match: {a} vs {b}");
    }

    #[test]
    fn empty_pattern_scores_zero() {
        assert_eq!(s("", "anything"), Some(0));
    }

    #[test]
    fn separator_boundary_boosts() {
        // "cfg" right after '/' should beat a mid-word occurrence.
        let boundary = s("cfg", "app/cfgfile").unwrap();
        let mid = s("cfg", "appcfgxx").unwrap();
        assert!(boundary >= mid);
    }
}

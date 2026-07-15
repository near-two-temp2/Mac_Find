//! 64-bit 字母 bitmask 编码 —— 混合引擎 Phase 1 O(1) 预过滤的核心。
//!
//! 编码方案与 Road_B 保持一致（对齐 Cling，见 open-source-analysis.md §3.3）：
//! ```text
//! Bits 0-25:  字母 a-z
//! Bits 26-35: 数字 0-9
//! Bit 36:     '.'（点）
//! Bit 37:     '-'（连字符）
//! Bit 38:     '_'（下划线）
//! 其余字符    忽略（不置位）
//! ```
//!
//! 查询时先算出查询串的 `combined_mask`，再用
//! `entry_mask & combined_mask != combined_mask` 一条指令排除不可能候选（无假阴性）。

/// 对单个字节置位，返回该字节对应的 mask 贡献（入参应为小写字节）。
#[inline(always)]
pub fn byte_bit(b: u8) -> u64 {
    match b {
        b'a'..=b'z' => 1u64 << (b - b'a'),
        b'0'..=b'9' => 1u64 << (26 + (b - b'0')),
        b'.' => 1u64 << 36,
        b'-' => 1u64 << 37,
        b'_' => 1u64 << 38,
        _ => 0,
    }
}

/// 计算一段（已小写）字节的字母 bitmask。
#[inline]
pub fn mask_of(lower_bytes: &[u8]) -> u64 {
    let mut m = 0u64;
    for &b in lower_bytes {
        m |= byte_bit(b);
    }
    m
}

/// Phase 1 预过滤：`entry` 是否可能包含 `query` 的全部字符。
///
/// 若返回 false，则该 entry 一定不匹配，可安全跳过。
#[inline(always)]
pub fn could_contain(entry_mask: u64, query_mask: u64) -> bool {
    entry_mask & query_mask == query_mask
}

/// 计算词边界位图：路径中每个「词起始」位置置 1（最多记录前 64 个字节）。
///
/// 词起始定义（用于 fzf 边界奖励）：
///   - 位置 0；
///   - 前一个字符是分隔符（`/ . - _ 空格`）。
///
/// 因为索引里存的是全小写字节，camelCase 边界在这里退化，故只按分隔符判定。
pub fn word_boundaries(lower_bytes: &[u8]) -> u64 {
    let mut b = 0u64;
    let mut prev_sep = true; // 视首字符前为分隔
    for (i, &c) in lower_bytes.iter().enumerate() {
        if i >= 64 {
            break;
        }
        let is_word = c.is_ascii_alphanumeric();
        if is_word && prev_sep {
            b |= 1u64 << i;
        }
        prev_sep = matches!(c, b'/' | b'.' | b'-' | b'_' | b' ');
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_roundtrip() {
        let m = mask_of(b"abc.rs");
        assert!(could_contain(m, mask_of(b"a")));
        assert!(could_contain(m, mask_of(b"cr")));
        assert!(could_contain(m, mask_of(b"rs")));
        assert!(!could_contain(m, mask_of(b"x")));
    }

    #[test]
    fn boundaries() {
        // "src/main.rs" 词起始：s(0)、m(4)、r(9)
        let b = word_boundaries(b"src/main.rs");
        assert!(b & 1 != 0); // pos 0
        assert!(b & (1 << 4) != 0); // 'm' after '/'
        assert!(b & (1 << 9) != 0); // 'r' after '.'
    }
}

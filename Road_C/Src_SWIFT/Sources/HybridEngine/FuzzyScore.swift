import Foundation

/// fzf-style fuzzy scoring over already-lowercased byte buffers.
///
/// A compact take on the algorithm sketched in open-source-analysis.md §3.4:
///   1. Anchor enumeration — every occurrence of the first pattern byte.
///   2. Forward greedy match from each anchor.
///   3. Score with boundary/contiguity bonuses and gap penalties.
///   4. Keep the best-scoring anchor.
///
/// Boundaries are supplied as a 64-bit bitmap over the basename (bit i set ⇒
/// text byte at `boundariesOffset + i` starts a word). Matching a pattern byte
/// on a boundary earns a bonus, which is what makes "am" rank `AppManager`
/// above `teamwork`.
public enum FuzzyScore {

    // Scoring weights (tuned to mirror Cling's relative ordering).
    static let scoreMatch          = 16
    static let scoreContiguous     = 4
    static let bonusBoundary       = 8
    static let bonusFirstChar      = 8      // extra when the first pattern byte hits a boundary
    static let penaltyGapStart     = -3
    static let penaltyGapExtend    = -1

    // Tier bonuses that make *literal* matches dominate scattered fuzzy hits.
    // These are deliberately large relative to the per-character weights above so
    // that, e.g., an exact `temp_test` basename can never be out-ranked by a
    // scattered subsequence like `contemplate_stest`. See §"精确/子串置顶".
    static let tierExact           = 100_000   // whole text == pattern
    static let tierPrefix          = 40_000    // text starts with pattern
    static let tierWordStart       = 20_000    // pattern sits at a word boundary
    static let tierSubstring       = 10_000    // pattern is a contiguous substring
    // (no tier bonus ⇒ plain scattered subsequence, ranks below all of the above)

    /// Score with the literal-match tiers layered on top of the fzf score.
    /// `text`/`pattern` are lowercased; `boundaries` marks word starts (bit i ⇒
    /// text byte i begins a word). Returns nil for a non-subsequence.
    ///
    /// The returned score is `tier + fuzzy`, so ordering is: exact > prefix >
    /// word-start-substring > substring > scattered. Within a tier the fzf score
    /// (contiguity/boundary/gap) still discriminates.
    public static func scoreRanked(
        pattern: UnsafeBufferPointer<UInt8>,
        text: UnsafeBufferPointer<UInt8>,
        boundaries: UInt64,
        boundariesOffset: Int
    ) -> (score: Int, start: Int, end: Int)? {
        guard let base = score(pattern: pattern, text: text,
                               boundaries: boundaries, boundariesOffset: boundariesOffset)
        else { return nil }

        let tier = literalTier(pattern: pattern, text: text,
                               boundaries: boundaries, boundariesOffset: boundariesOffset)
        return (base.score + tier, base.start, base.end)
    }

    /// Detect the strongest literal relationship between `pattern` and `text`.
    /// Returns the matching tier bonus (0 for a plain scattered match).
    @inline(__always)
    static func literalTier(
        pattern: UnsafeBufferPointer<UInt8>,
        text: UnsafeBufferPointer<UInt8>,
        boundaries: UInt64,
        boundariesOffset: Int
    ) -> Int {
        let pLen = pattern.count
        let tLen = text.count
        if pLen == 0 || pLen > tLen { return 0 }

        // Find the first contiguous occurrence of `pattern` inside `text`.
        var at = -1
        var i = 0
        while i <= tLen - pLen {
            var k = 0
            while k < pLen && text[i + k] == pattern[k] { k += 1 }
            if k == pLen { at = i; break }
            i += 1
        }
        if at < 0 { return 0 }                       // not a substring at all

        if at == 0 {
            return tLen == pLen ? tierExact : tierPrefix
        }
        // Substring elsewhere — extra credit if it starts on a word boundary.
        let bit = at - boundariesOffset
        let onBoundary = bit >= 0 && bit < 64 && (boundaries & (1 << UInt64(bit))) != 0
        return onBoundary ? tierWordStart : tierSubstring
    }

    /// Score `pattern` against `text` (both lowercased). Returns nil if not a
    /// subsequence match. `boundaries`/`boundariesOffset` describe word starts
    /// as in the doc-comment above.
    public static func score(
        pattern: UnsafeBufferPointer<UInt8>,
        text: UnsafeBufferPointer<UInt8>,
        boundaries: UInt64,
        boundariesOffset: Int
    ) -> (score: Int, start: Int, end: Int)? {
        let pLen = pattern.count
        let tLen = text.count
        if pLen == 0 { return (0, 0, 0) }
        if pLen > tLen { return nil }

        let first = pattern[0]
        var best: (score: Int, start: Int, end: Int)? = nil

        // Enumerate anchors: positions in text equal to the first pattern byte.
        var anchor = 0
        var anchorsTried = 0
        while anchor <= tLen - pLen && anchorsTried < 32 {
            if text[anchor] != first {
                anchor += 1
                continue
            }
            anchorsTried += 1

            if let s = scoreFromAnchor(
                pattern: pattern, text: text, anchor: anchor,
                boundaries: boundaries, boundariesOffset: boundariesOffset
            ) {
                if best == nil || s.score > best!.score {
                    best = s
                }
            }
            anchor += 1
        }
        return best
    }

    /// Greedy forward match starting at `anchor`, accumulating the score.
    @inline(__always)
    private static func scoreFromAnchor(
        pattern: UnsafeBufferPointer<UInt8>,
        text: UnsafeBufferPointer<UInt8>,
        anchor: Int,
        boundaries: UInt64,
        boundariesOffset: Int
    ) -> (score: Int, start: Int, end: Int)? {
        let pLen = pattern.count
        let tLen = text.count

        var score = 0
        var pi = 0
        var ti = anchor
        var prevMatchIndex = -2   // so the first match is never "contiguous"
        var inGap = false

        while pi < pLen {
            // Find pattern[pi] in text starting at ti.
            let needle = pattern[pi]
            var found = -1
            var j = ti
            while j < tLen {
                if text[j] == needle { found = j; break }
                j += 1
            }
            if found == -1 { return nil }

            score += scoreMatch

            let onBoundary = isBoundary(found, boundaries: boundaries, offset: boundariesOffset)
            if onBoundary {
                score += bonusBoundary
                if pi == 0 { score += bonusFirstChar }
            }

            if found == prevMatchIndex + 1 {
                score += scoreContiguous
                inGap = false
            } else if prevMatchIndex >= 0 {
                // A gap between consecutive matched chars.
                score += inGap ? penaltyGapExtend : penaltyGapStart
                let gap = found - prevMatchIndex - 1
                if gap > 1 { score += penaltyGapExtend * (gap - 1) }
                inGap = true
            }

            prevMatchIndex = found
            ti = found + 1
            pi += 1
        }

        let start = anchor
        let end = prevMatchIndex + 1
        return (score, start, end)
    }

    @inline(__always)
    private static func isBoundary(_ textIndex: Int, boundaries: UInt64, offset: Int) -> Bool {
        let bit = textIndex - offset
        if bit < 0 || bit >= 64 { return false }
        return (boundaries & (1 << UInt64(bit))) != 0
    }
}

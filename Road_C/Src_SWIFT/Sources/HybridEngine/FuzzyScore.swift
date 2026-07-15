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

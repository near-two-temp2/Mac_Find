import Foundation

/// fzf-style fuzzy scorer operating on raw lowercased byte buffers.
///
/// Algorithm (mirrors the analysis doc §3.4):
///   1. Anchor enumeration — SIMD scan for every occurrence of the first
///      pattern byte (capped at 32 anchors).
///   2. Forward greedy match from each anchor.
///   3. Reverse tightening within the matched window for the most compact span.
///   4. Score: char match + contiguity + boundary + first-char bonuses,
///      minus gap penalties.
public enum FuzzyScorer {

    // Scoring constants (see analysis doc §3.4 table).
    static let scoreMatch = 16
    static let scoreContiguous = 4
    static let bonusBoundary = 8
    static let bonusFirstMul = 2
    static let penaltyGapStart = -3
    static let penaltyGapExtension = -1

    public struct Match {
        public let score: Int
        public let start: Int
        public let end: Int
    }

    /// Score `pattern` (lowercased) against `text` (lowercased). `boundaries` is
    /// the word-boundary bitmap for the *basename*, and `boundariesOffset` is
    /// where the basename starts within `text` so we can align bits to indices.
    public static func score(
        pattern: UnsafeBufferPointer<UInt8>,
        text: UnsafeBufferPointer<UInt8>,
        boundaries: UInt64,
        boundariesOffset: Int
    ) -> Match? {
        let m = pattern.count
        let n = text.count
        guard m > 0, n >= m else { return m == 0 ? Match(score: 0, start: 0, end: 0) : nil }

        let first = pattern[0]
        var bestScore = Int.min
        var bestStart = -1
        var bestEnd = -1

        var anchorCount = 0
        var pos = simdFindByte(text.baseAddress!, count: n, needle: first, from: 0)
        while pos >= 0 && anchorCount < 32 {
            anchorCount += 1
            if let (score, start, end) = matchFrom(
                anchor: pos, pattern: pattern, text: text,
                boundaries: boundaries, boundariesOffset: boundariesOffset
            ) {
                if score > bestScore {
                    bestScore = score
                    bestStart = start
                    bestEnd = end
                }
            }
            if pos + 1 >= n { break }
            pos = simdFindByte(text.baseAddress!, count: n, needle: first, from: pos + 1)
        }

        guard bestStart >= 0 else { return nil }
        return Match(score: bestScore, start: bestStart, end: bestEnd)
    }

    /// Forward greedy match starting at `anchor`, then reverse-tighten.
    private static func matchFrom(
        anchor: Int,
        pattern: UnsafeBufferPointer<UInt8>,
        text: UnsafeBufferPointer<UInt8>,
        boundaries: UInt64,
        boundariesOffset: Int
    ) -> (score: Int, start: Int, end: Int)? {
        let m = pattern.count
        let n = text.count

        // Forward pass: find the end index where all pattern chars are consumed.
        var pi = 0
        var ti = anchor
        while ti < n && pi < m {
            if text[ti] == pattern[pi] { pi += 1 }
            ti += 1
        }
        guard pi == m else { return nil } // pattern not fully contained
        let end = ti // exclusive

        // Reverse pass: tighten the start by matching backwards from `end`.
        var pj = m - 1
        var tj = end - 1
        var start = anchor
        while tj >= anchor && pj >= 0 {
            if text[tj] == pattern[pj] {
                if pj == 0 { start = tj }
                pj -= 1
            }
            tj -= 1
        }

        // Score the compact [start, end) window.
        var score = 0
        var pk = 0
        var prevMatched = -2
        var k = start
        while k < end && pk < m {
            if text[k] == pattern[pk] {
                var s = scoreMatch
                // Boundary bonus (only meaningful within the basename).
                let bnIdx = k - boundariesOffset
                if bnIdx >= 0 && bnIdx < 64 && (boundaries & (1 << UInt64(bnIdx))) != 0 {
                    s += bonusBoundary
                }
                // Contiguity bonus.
                if k == prevMatched + 1 {
                    s += scoreContiguous
                }
                // First-character bonus.
                if pk == 0 {
                    s *= bonusFirstMul
                }
                score += s
                prevMatched = k
                pk += 1
            } else if prevMatched >= 0 {
                // Inside a gap.
                score += (k == prevMatched + 1) ? penaltyGapStart : penaltyGapExtension
            }
            k += 1
        }

        // Shorter matched spans are better; nudge the score for compactness.
        score -= (end - start - m)
        return (score, start, end)
    }

    /// SIMD byte search: returns the index of `needle` at or after `from`, or -1.
    /// Scans 16 bytes per step, then handles the remainder scalar-wise.
    @inline(__always)
    static func simdFindByte(
        _ base: UnsafePointer<UInt8>, count: Int, needle: UInt8, from: Int
    ) -> Int {
        var i = from
        let needleVec = SIMD16<UInt8>(repeating: needle)
        while i + 16 <= count {
            let chunk = SIMD16<UInt8>(
                (0..<16).map { base[i + $0] }
            )
            let eq = chunk .== needleVec
            if any(eq) {
                // Find first set lane.
                for lane in 0..<16 where eq[lane] {
                    return i + lane
                }
            }
            i += 16
        }
        while i < count {
            if base[i] == needle { return i }
            i += 1
        }
        return -1
    }
}

// fzf.hpp — fzf-style fuzzy scorer for Phase-2 ranking.
//
// Ported in spirit from Cling's fuzzyScoreBytes (open-source-analysis.md §3.4):
// enumerate anchors on the first pattern byte, greedily match forward, tighten
// backward, then score with word-boundary / consecutive bonuses and gap
// penalties. Operates on already-lowercased bytes (both index and query are
// lowercased) so matching is case-insensitive.

#pragma once

#include <cstdint>
#include <cstddef>

namespace mff {

// Scoring weights (mirrors the table in the analysis report).
static constexpr int kScoreMatch      = 16; // per matched char
static constexpr int kScoreConsec     = 4;  // consecutive match bonus
static constexpr int kScoreBoundary   = 8;  // match at a word boundary
static constexpr int kScoreFirstBonus = 16; // extra if match starts at pos 0
static constexpr int kGapStart        = -3; // first skipped char in a gap
static constexpr int kGapExtend       = -1; // subsequent skipped chars

struct ScoreResult {
    bool matched = false;
    int  score   = 0;
    int  start   = 0; // byte index in text where the match window begins
    int  end     = 0; // one past the last matched byte
};

// True if `text[i]` sits on a word boundary (start, or preceded by a
// separator, or a lowercase->uppercase style transition encoded via the
// precomputed boundary bitmap when available). Here we derive boundaries from
// the bytes directly for CLI callers that lack the bitmap.
inline bool isBoundaryByte(const uint8_t* text, int i) {
    if (i == 0) return true;
    uint8_t prev = text[i - 1];
    return prev == '/' || prev == '_' || prev == '-' || prev == '.' ||
           prev == ' ';
}

// Score `pattern` (length plen) against `text` (length tlen). Both must be
// lowercase. Returns matched=false when pattern is not a subsequence of text.
// `boundaries` is an optional word-boundary bitmap for `text` (bit i set =>
// text[i] is a boundary); pass 0 with `hasBoundaries=false` to derive it.
inline ScoreResult fuzzyScore(const uint8_t* pattern, size_t plen,
                              const uint8_t* text, size_t tlen,
                              uint64_t boundaries = 0,
                              bool hasBoundaries = false) {
    ScoreResult r;
    if (plen == 0) { r.matched = true; return r; }
    if (tlen == 0 || plen > tlen) return r;

    auto boundaryAt = [&](int i) -> bool {
        if (hasBoundaries && i < 64) return (boundaries >> i) & 1ULL;
        return isBoundaryByte(text, i);
    };

    int best = -1;
    ScoreResult bestRes;

    // Enumerate anchors: every position where text matches pattern[0].
    for (size_t anchor = 0; anchor + plen <= tlen; ++anchor) {
        if (text[anchor] != pattern[0]) continue;

        // Forward greedy match from this anchor.
        int score = 0;
        size_t pi = 0;
        size_t ti = anchor;
        int prevMatch = -1;
        int matchStart = -1;

        while (pi < plen && ti < tlen) {
            if (text[ti] == pattern[pi]) {
                if (matchStart < 0) matchStart = (int)ti;
                score += kScoreMatch;
                if (boundaryAt((int)ti)) score += kScoreBoundary;
                if (ti == 0) score += kScoreFirstBonus;
                if (prevMatch >= 0 && (int)ti == prevMatch + 1)
                    score += kScoreConsec;
                prevMatch = (int)ti;
                ++pi;
                ++ti;
            } else {
                // Gap: penalize the run of skipped bytes.
                score += kGapStart;
                ++ti;
                // Extend penalty for each further skipped byte until the next
                // potential match (kept simple: one extend per skip).
                score += kGapExtend;
            }
        }

        if (pi == plen) { // full pattern consumed => it's a real match
            if (score > best) {
                best = score;
                bestRes.matched = true;
                bestRes.score = score;
                bestRes.start = matchStart;
                bestRes.end = prevMatch + 1;
            }
        }
    }

    return bestRes;
}

} // namespace mff

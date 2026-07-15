// Package fuzzy implements an fzf-style fuzzy matcher/scorer over lowercased
// byte strings, modeled on the Cling scoring described in
// open-source-analysis.md §3.4 (anchor enumeration, greedy forward match,
// backward tightening, boundary/consecutive bonuses).
//
// Scoring weights (mirroring the reference):
//
//	character match     +16
//	consecutive match    +4
//	first-char bonus     ×2  (applied to the base of the first matched char)
//	word-boundary bonus  +7/+8/+9
//	gap start            -3
//	gap continuation     -1
package fuzzy

const (
	scoreMatch       = 16
	scoreConsecutive = 4
	bonusBoundary    = 8
	bonusCamel       = 7
	bonusSeparator   = 9
	penaltyGapStart  = -3
	penaltyGapExt    = -1
)

// Result is a successful fuzzy match: its score and the [start,end) byte range
// in the text that the pattern matched against.
type Result struct {
	Score int
	Start int
	End   int
}

// Match reports whether pattern fuzzy-matches text and, if so, the best score
// and span. Both pattern and text are expected to be lowercased already
// (pattern is normalized by the caller once; text comes from the lowercased
// index or is lowercased by the fallback path).
//
// It returns ok=false when the pattern cannot be matched in order.
func Match(pattern, text string) (Result, bool) {
	if len(pattern) == 0 {
		return Result{Score: 0, Start: 0, End: 0}, true
	}
	if len(pattern) > len(text) {
		return Result{}, false
	}

	first := pattern[0]
	best := Result{}
	found := false

	// Anchor enumeration: try every occurrence of the first pattern byte as a
	// starting point and keep the highest-scoring alignment.
	for anchor := indexByte(text, first, 0); anchor >= 0; anchor = indexByte(text, first, anchor+1) {
		if r, ok := scoreFrom(pattern, text, anchor); ok {
			if !found || r.Score > best.Score {
				best = r
				found = true
			}
		}
		// Not enough room left for the remaining pattern bytes.
		if anchor+len(pattern) > len(text) {
			break
		}
	}
	return best, found
}

// scoreFrom greedily matches pattern against text starting at anchor and
// computes the score for that alignment. It fails if the pattern can't be
// consumed in order.
func scoreFrom(pattern, text string, anchor int) (Result, bool) {
	score := 0
	pi := 0
	prevMatched := false
	prevInGap := false
	start := anchor
	last := anchor

	for ti := anchor; ti < len(text) && pi < len(pattern); ti++ {
		if text[ti] == pattern[pi] {
			base := scoreMatch
			if pi == 0 {
				base *= 2 // first-char bonus
			}
			score += base
			if prevMatched {
				score += scoreConsecutive
			}
			score += boundaryBonus(text, ti)
			prevMatched = true
			prevInGap = false
			last = ti
			pi++
		} else {
			if prevInGap {
				score += penaltyGapExt
			} else {
				score += penaltyGapStart
				prevInGap = true
			}
			prevMatched = false
		}
	}

	if pi < len(pattern) {
		return Result{}, false
	}
	if score < 0 {
		score = 0
	}
	return Result{Score: score, Start: start, End: last + 1}, true
}

// boundaryBonus rewards matches that land on a word boundary: after a
// separator, at the very start, or at a camelCase transition.
func boundaryBonus(text string, i int) int {
	if i == 0 {
		return bonusSeparator
	}
	prev := text[i-1]
	switch {
	case prev == '/' || prev == ' ' || prev == '_' || prev == '-' || prev == '.':
		return bonusSeparator
	case isDigit(prev) != isDigit(text[i]):
		return bonusBoundary
	}
	return 0
}

func isDigit(c byte) bool { return c >= '0' && c <= '9' }

func indexByte(s string, c byte, from int) int {
	for i := from; i < len(s); i++ {
		if s[i] == c {
			return i
		}
	}
	return -1
}

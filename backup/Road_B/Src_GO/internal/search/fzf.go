// Package search implements the two-phase Road_B query pipeline over an
// index.Index: Phase 1 does a goroutine-parallel bitmask + extension
// pre-filter, Phase 2 runs an fzf-style fuzzy score on the survivors.
//
// The scoring model follows open-source-analysis.md §3.4:
//
//	character match     +16
//	consecutive match    +4
//	first-char bonus     ×2 (applied to the first matched char's base)
//	word-boundary bonus  +7 camelCase / +8 space / +9 separator
//	gap start            -3
//	gap continuation     -1
package search

// Scoring constants (see package doc).
const (
	scoreMatch       = 16
	scoreConsecutive = 4
	bonusBoundary    = 8 // generic boundary (after '/', '_', '-', '.', ' ')
	bonusCamel       = 7 // lower→Upper transition; on lowercased text approximated
	penaltyGapStart  = -3
	penaltyGapExt    = -1
)

// isBoundaryByte reports whether b is a separator that makes the following
// character a word boundary.
func isBoundaryByte(b byte) bool {
	switch b {
	case '/', '_', '-', '.', ' ':
		return true
	default:
		return false
	}
}

// fuzzyScore scores pattern against text (both lowercase bytes). It returns
// the score and whether every pattern byte was matched in order. A pattern
// that does not appear as a subsequence scores as (0, false).
//
// The algorithm mirrors Cling's approach at a smaller scale: for each
// occurrence of the first pattern byte we run a greedy forward match, award
// boundary/consecutive bonuses and gap penalties, and keep the best-scoring
// anchor. Anchor enumeration is capped so pathological inputs stay bounded.
func fuzzyScore(pattern, text []byte) (int, bool) {
	if len(pattern) == 0 {
		return 0, true
	}
	if len(pattern) > len(text) {
		return 0, false
	}

	first := pattern[0]
	best := 0
	found := false

	const maxAnchors = 32
	anchors := 0

	for start := 0; start < len(text); start++ {
		if text[start] != first {
			continue
		}
		anchors++
		if anchors > maxAnchors {
			break
		}

		s, ok := scoreFromAnchor(pattern, text, start)
		if ok {
			found = true
			if s > best {
				best = s
			}
		}
	}

	return best, found
}

// scoreFromAnchor greedily matches pattern against text starting at the given
// anchor index (where text[anchor] == pattern[0]) and returns the resulting
// score. It reports false if the pattern cannot be completed from here.
func scoreFromAnchor(pattern, text []byte, anchor int) (int, bool) {
	score := 0
	ti := anchor
	prevMatched := -2 // index in text of the previously matched char

	for pi := 0; pi < len(pattern); pi++ {
		pc := pattern[pi]

		// Advance ti to the next occurrence of pc.
		for ti < len(text) && text[ti] != pc {
			ti++
		}
		if ti >= len(text) {
			return 0, false
		}

		base := scoreMatch

		// Boundary bonus: the matched char begins a word.
		if ti == 0 || isBoundaryByte(text[ti-1]) {
			base += bonusBoundary
		}

		if prevMatched == ti-1 {
			// Consecutive with the previous match: reward, no gap penalty.
			base += scoreConsecutive
		} else if prevMatched >= 0 {
			// There was a gap between matches.
			gap := ti - prevMatched - 1
			base += penaltyGapStart
			if gap > 1 {
				base += penaltyGapExt * (gap - 1)
			}
		}

		if pi == 0 {
			base *= 2 // first-char bonus
		}

		score += base
		prevMatched = ti
		ti++
	}

	return score, true
}

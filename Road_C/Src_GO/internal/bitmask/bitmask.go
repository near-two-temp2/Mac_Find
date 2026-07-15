// Package bitmask implements the Cling-style 64-bit character bitmask used for
// O(1) candidate pre-filtering. Each set bit records that a given character
// class appears somewhere in the (lowercased) string.
//
// Bit layout (see open-source-analysis.md §3.3):
//
//	Bits 0-25:  letters a-z
//	Bits 26-35: digits 0-9
//	Bit 36:     '.'
//	Bit 37:     '-'
//	Bit 38:     '_'
//
// A query can only match an entry if every bit of the query's mask is also set
// in the entry's mask, i.e. `entryMask & queryMask == queryMask`. This lets us
// discard the vast majority of non-matching entries with a single AND.
package bitmask

const (
	bitDot        = 36
	bitDash       = 37
	bitUnderscore = 38
)

// Of computes the character-class bitmask for s. The input is assumed to be
// already lowercased (the index stores lowercased paths); any byte outside the
// tracked classes is ignored.
func Of(s string) uint64 {
	var m uint64
	for i := 0; i < len(s); i++ {
		m |= bitOf(s[i])
	}
	return m
}

// OfLower lowercases ASCII letters on the fly, for callers that hold mixed-case
// input (e.g. a raw user query).
func OfLower(s string) uint64 {
	var m uint64
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c >= 'A' && c <= 'Z' {
			c += 'a' - 'A'
		}
		m |= bitOf(c)
	}
	return m
}

func bitOf(c byte) uint64 {
	switch {
	case c >= 'a' && c <= 'z':
		return 1 << (c - 'a')
	case c >= '0' && c <= '9':
		return 1 << (26 + (c - '0'))
	case c == '.':
		return 1 << bitDot
	case c == '-':
		return 1 << bitDash
	case c == '_':
		return 1 << bitUnderscore
	}
	return 0
}

// Matches reports whether an entry with mask entry could contain every
// character class required by query.
func Matches(entry, query uint64) bool {
	return entry&query == query
}

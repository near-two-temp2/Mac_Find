// Package index implements a Cling-inspired binary file-name index for
// Road_B: parallel arrays, a per-entry uint64 letter bitmask for O(1)
// pre-filtering, and an extension-ID column for fast type matching.
//
// The bitmask encoding follows open-source-analysis.md §3.3:
//
//	Bits 0-25:  letters a-z
//	Bits 26-35: digits 0-9
//	Bit 36:     '.'
//	Bit 37:     '-'
//	Bit 38:     '_'
package index

// MaskFor computes the presence bitmask for a byte slice. The input is
// expected to already be lowercase (paths are stored lowercased). Any byte
// that does not map to a tracked class is ignored, which is fine: the mask
// is a conservative pre-filter, never the final matcher.
func MaskFor(b []byte) uint64 {
	var m uint64
	for _, c := range b {
		m |= bitForByte(c)
	}
	return m
}

// MaskForString is the string convenience form of MaskFor. Query strings are
// lowercased by the caller before this is used.
func MaskForString(s string) uint64 {
	var m uint64
	for i := 0; i < len(s); i++ {
		m |= bitForByte(s[i])
	}
	return m
}

// bitForByte returns the single-bit mask for one byte, or 0 for untracked
// bytes (e.g. '/', spaces, non-ASCII).
func bitForByte(c byte) uint64 {
	switch {
	case c >= 'a' && c <= 'z':
		return 1 << (c - 'a') // bits 0-25
	case c >= '0' && c <= '9':
		return 1 << (26 + (c - '0')) // bits 26-35
	case c == '.':
		return 1 << 36
	case c == '-':
		return 1 << 37
	case c == '_':
		return 1 << 38
	default:
		return 0
	}
}

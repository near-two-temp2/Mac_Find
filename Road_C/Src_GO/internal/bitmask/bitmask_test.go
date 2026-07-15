package bitmask

import "testing"

func TestMatchesPrefilter(t *testing.T) {
	entry := Of("readme.md")
	// Query chars all present -> possible match.
	if !Matches(entry, Of("read")) {
		t.Error("'read' should pass the prefilter for 'readme.md'")
	}
	if !Matches(entry, OfLower("ME")) {
		t.Error("case-insensitive 'ME' should pass for 'readme.md'")
	}
	// Query needs a char the entry lacks -> guaranteed non-match.
	if Matches(entry, Of("xyz")) {
		t.Error("'xyz' must be rejected: 'x' not in 'readme.md'")
	}
}

func TestPunctuationBits(t *testing.T) {
	m := Of("a-b_c.d")
	for _, q := range []string{"-", "_", "."} {
		if !Matches(m, Of(q)) {
			t.Errorf("punctuation %q should be tracked", q)
		}
	}
}

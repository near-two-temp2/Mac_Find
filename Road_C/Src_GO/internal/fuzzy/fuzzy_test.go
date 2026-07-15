package fuzzy

import "testing"

func TestMatchBasic(t *testing.T) {
	cases := []struct {
		pattern, text string
		want          bool
	}{
		{"go", "main.go", true},
		{"mg", "main.go", true},   // in-order fuzzy across a gap
		{"xyz", "main.go", false}, // missing chars
		{"", "anything", true},    // empty pattern always matches
		{"main", "main.go", true},
		{"oog", "google", true},  // o,o,g present in order
		{"ggg", "google", false}, // only two g's
	}
	for _, c := range cases {
		_, ok := Match(c.pattern, c.text)
		if ok != c.want {
			t.Errorf("Match(%q,%q)=%v want %v", c.pattern, c.text, ok, c.want)
		}
	}
}

func TestBoundaryBeatsMiddle(t *testing.T) {
	// A match right after a separator should outscore a mid-word match.
	boundary, ok1 := Match("go", "foo/gopher")
	middle, ok2 := Match("go", "argonaut")
	if !ok1 || !ok2 {
		t.Fatalf("expected both to match: %v %v", ok1, ok2)
	}
	if boundary.Score <= middle.Score {
		t.Errorf("boundary score %d should beat middle score %d", boundary.Score, middle.Score)
	}
}

package index

import (
	"os"
	"path/filepath"
	"testing"
)

// TestBuildOpenSearch exercises the full index lifecycle: build a temp tree,
// serialize an index, reopen it, and confirm the two-phase search finds files
// by basename and rejects impossible queries.
func TestBuildOpenSearch(t *testing.T) {
	root := t.TempDir()
	files := []string{"alpha.txt", "sub/beta.go", "sub/deep/gamma.md"}
	for _, f := range files {
		p := filepath.Join(root, f)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte("x"), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	idxPath := filepath.Join(t.TempDir(), "test.idx")
	n, err := Build([]string{root}, idxPath)
	if err != nil {
		t.Fatalf("Build: %v", err)
	}
	if n == 0 {
		t.Fatal("expected non-zero entries")
	}

	ix, err := Open(idxPath)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	if ix.Count() != n {
		t.Errorf("Count()=%d want %d", ix.Count(), n)
	}

	if got := ix.Search("beta", 10); len(got) == 0 {
		t.Error("expected a hit for 'beta'")
	}
	if got := ix.Search("gamma", 10); len(got) == 0 {
		t.Error("expected a hit for 'gamma'")
	}
	if got := ix.Search("zzzqqq", 10); len(got) != 0 {
		t.Errorf("expected no hits for 'zzzqqq', got %d", len(got))
	}
}

// TestOpenRejectsCorrupt confirms a bad/short file triggers ErrBadIndex so the
// engine will fall back to searchfs().
func TestOpenRejectsCorrupt(t *testing.T) {
	p := filepath.Join(t.TempDir(), "bad.idx")
	if err := os.WriteFile(p, []byte("not an index"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := Open(p); err == nil {
		t.Error("Open should reject a corrupt file")
	}
	if _, err := Open(filepath.Join(t.TempDir(), "missing.idx")); err == nil {
		t.Error("Open should reject a missing file")
	}
}

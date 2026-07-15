package engine

import (
	"os"
	"path/filepath"
	"testing"

	"macfind/roadc/internal/index"
)

// TestHybridPrefersIndex builds a real index and confirms the engine serves
// results from it, labeled SourceIndex.
func TestHybridPrefersIndex(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "hybrid_target.txt"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	idxPath := filepath.Join(t.TempDir(), "e.idx")
	if _, err := index.Build([]string{root}, idxPath); err != nil {
		t.Fatal(err)
	}

	e := New(idxPath)
	if !e.HasIndex() {
		t.Fatal("expected index to load")
	}
	res, src := e.Search("hybrid", 10)
	if src != SourceIndex {
		t.Errorf("source=%v want SourceIndex", src)
	}
	if len(res) == 0 {
		t.Error("expected index hit for 'hybrid'")
	}
}

// TestFallbackWhenNoIndex confirms that with no index loaded, the engine reports
// the searchfs fallback path (result contents depend on the platform).
func TestFallbackWhenNoIndex(t *testing.T) {
	e := New(filepath.Join(t.TempDir(), "does-not-exist.idx"))
	if e.HasIndex() {
		t.Fatal("expected no index")
	}
	_, src := e.Search("anything", 5)
	if src != SourceSearchFS {
		t.Errorf("source=%v want SourceSearchFS", src)
	}
}

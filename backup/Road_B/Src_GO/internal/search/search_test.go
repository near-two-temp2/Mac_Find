package search

import (
	"testing"

	"macfind/internal/index"
)

// buildTestIndex returns a small in-memory index over a fixed path set.
func buildTestIndex(t *testing.T) *index.Index {
	t.Helper()
	b := index.NewBuilder()
	paths := []struct {
		p     string
		isDir bool
	}{
		{"/Users/alice/projects/macfind/main.go", false},
		{"/Users/alice/projects/macfind/README.md", false},
		{"/Users/alice/Documents/report.pdf", false},
		{"/Users/alice/Documents", true},
		{"/Users/alice/photos/beach.jpg", false},
		{"/Applications/Safari.app", true},
	}
	for _, e := range paths {
		b.Add(e.p, e.isDir)
	}
	return b.Build()
}

func TestQueryFindsBasenameMatch(t *testing.T) {
	ix := buildTestIndex(t)
	res := Query(ix, "main", Options{Limit: 10})
	if len(res) == 0 {
		t.Fatal("expected at least one result for 'main'")
	}
	if res[0].Path != "/users/alice/projects/macfind/main.go" {
		t.Fatalf("top result = %q, want main.go path", res[0].Path)
	}
}

func TestQueryFuzzy(t *testing.T) {
	ix := buildTestIndex(t)
	// "rdme" is a subsequence of "readme".
	res := Query(ix, "rdme", Options{Limit: 10})
	found := false
	for _, r := range res {
		if r.Path == "/users/alice/projects/macfind/readme.md" {
			found = true
		}
	}
	if !found {
		t.Fatalf("fuzzy 'rdme' did not match README.md; got %d results", len(res))
	}
}

func TestBitmaskRejects(t *testing.T) {
	ix := buildTestIndex(t)
	// 'z' appears in no path; combined mask must reject everything.
	res := Query(ix, "zzz", Options{Limit: 10})
	if len(res) != 0 {
		t.Fatalf("expected no results for 'zzz', got %d", len(res))
	}
}

func TestExtensionFilter(t *testing.T) {
	ix := buildTestIndex(t)
	res := Query(ix, "", Options{Limit: 10, Ext: "pdf"})
	if len(res) != 1 {
		t.Fatalf("expected 1 pdf, got %d", len(res))
	}
	if res[0].Path != "/users/alice/documents/report.pdf" {
		t.Fatalf("pdf filter returned %q", res[0].Path)
	}
}

func TestDirsOnly(t *testing.T) {
	ix := buildTestIndex(t)
	res := Query(ix, "", Options{Limit: 10, DirsOnly: true})
	for _, r := range res {
		if !r.IsDir {
			t.Fatalf("DirsOnly returned a file: %q", r.Path)
		}
	}
	if len(res) != 2 {
		t.Fatalf("expected 2 directories, got %d", len(res))
	}
}

func TestSaveLoadRoundTrip(t *testing.T) {
	ix := buildTestIndex(t)
	path := t.TempDir() + "/test.idx"
	if err := ix.Save(path); err != nil {
		t.Fatalf("save: %v", err)
	}
	loaded, err := index.Open(path)
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer loaded.Close()

	if loaded.Count != ix.Count {
		t.Fatalf("count mismatch: got %d want %d", loaded.Count, ix.Count)
	}
	res := Query(loaded, "main", Options{Limit: 10})
	if len(res) == 0 || res[0].Path != "/users/alice/projects/macfind/main.go" {
		t.Fatalf("round-tripped index did not return main.go; got %d results", len(res))
	}
}

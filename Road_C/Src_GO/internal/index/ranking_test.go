package index

import (
	"os"
	"path/filepath"
	"testing"
)

// TestExactBasenameRanksFirst is the SEARCH_TEST_BASELINE.md acceptance check:
// searching "temp_test" must put the directory literally named temp_test at the
// top, above fuzzy-subsequence noise like "vscode_pytest" (which contains the
// letters t-e-m-p-_-t-e-s-t scattered as a subsequence) and above a deeper
// basename hit. We build a tree that reproduces that noise and assert ordering.
func TestExactBasenameRanksFirst(t *testing.T) {
	root := t.TempDir()
	// The exact target plus decoys that a naive fzf scorer would surface.
	dirs := []string{
		"temp_test",                      // exact target
		"a/b/c/nested_temp_test_folder",  // substring but longer/deeper
		"noise/vscode_pytest",            // scattered subsequence noise
		"noise/testing_temporary_helper", // more scattered noise
	}
	for _, d := range dirs {
		if err := os.MkdirAll(filepath.Join(root, d), 0o755); err != nil {
			t.Fatal(err)
		}
	}

	idxPath := filepath.Join(t.TempDir(), "rank.idx")
	if _, err := Build([]string{root}, idxPath); err != nil {
		t.Fatalf("Build: %v", err)
	}
	ix, err := Open(idxPath)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}

	got := ix.Search("temp_test", 20)
	if len(got) == 0 {
		t.Fatal("expected hits for temp_test")
	}
	// #1 must be the exact-named directory.
	wantSuffix := string(filepath.Separator) + "temp_test"
	if filepath.Base(got[0].Path) != "temp_test" {
		t.Fatalf("top hit = %q, want basename temp_test (suffix %q)", got[0].Path, wantSuffix)
	}
	// And it must strictly outscore the scattered-subsequence decoys.
	top := got[0].Score
	for _, r := range got[1:] {
		if filepath.Base(r.Path) == "vscode_pytest" && r.Score >= top {
			t.Errorf("decoy vscode_pytest scored %d >= exact temp_test %d", r.Score, top)
		}
	}
}

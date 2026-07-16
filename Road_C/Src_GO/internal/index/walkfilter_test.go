package index

import "testing"

// TestSkipperExcludesNetworkAndStatic verifies the walk pruning rules: known
// rclone→B2 mounts and CloudStorage are always excluded, and any path on (or
// under) a detected network mount point is pruned — while ordinary local paths
// pass. This is the guard that keeps index builds off network volumes
// (SEARCH_TEST_BASELINE.md), so it must reject the known-bad prefixes exactly.
func TestSkipperExcludesNetworkAndStatic(t *testing.T) {
	s := &skipper{
		staticPrefixes: []string{
			"/Volumes/Disk/h2-",
			"/Volumes/Disk/h2_",
			"/Users/me/Library/CloudStorage",
		},
		networkPoints: []string{"/Volumes/Disk/h2_bu_01_b2"},
	}

	skip := []string{
		"/Volumes/Disk/h2-bu-01",
		"/Volumes/Disk/h2_bu_01_b2",
		"/Volumes/Disk/h2_bu_01_b2/deep/child", // under a network mount
		"/Volumes/Disk/h2_open_rsh",
		"/Users/me/Library/CloudStorage/GoogleDrive-x",
	}
	for _, p := range skip {
		if !s.shouldSkip(p) {
			t.Errorf("shouldSkip(%q) = false, want true", p)
		}
	}

	keep := []string{
		"/Users/me/temp_test",
		"/Applications",
		"/Volumes/MacDisk/Users/Shared/temp_test",
		"/Volumes/Disk/local_folder", // local sibling of the h2-* mounts
	}
	for _, p := range keep {
		if s.shouldSkip(p) {
			t.Errorf("shouldSkip(%q) = true, want false", p)
		}
	}
}

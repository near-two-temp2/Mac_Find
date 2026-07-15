//go:build !darwin

package engine

import "fmt"

// This stub lets the package (and the CLI smoke test) compile and vet on
// non-darwin platforms. searchfs(2) is macOS-only, so Search is inert here.

func defaultVolumes() []string {
	return []string{"/"}
}

// Search always fails on non-darwin builds: searchfs(2) does not exist.
func Search(opts Options) ([]Result, error) {
	return nil, fmt.Errorf("searchfs is only available on macOS/darwin")
}

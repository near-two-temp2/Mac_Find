package index

import (
	"os"
	"path/filepath"
)

// DefaultPath returns the standard on-disk location for the index, mirroring
// where Cling keeps its .idx files:
//
//	~/Library/Caches/com.macfind.roadb.go/index.idx
//
// The parent directory is created if needed.
func DefaultPath() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	dir := filepath.Join(home, "Library", "Caches", "com.macfind.roadb.go")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return "", err
	}
	return filepath.Join(dir, "index.idx"), nil
}

// DefaultRoots returns the directories scanned by default: the user's home
// directory plus /Applications, which together cover the vast majority of a
// user's searchable files without needing root.
func DefaultRoots() []string {
	roots := []string{}
	if home, err := os.UserHomeDir(); err == nil {
		roots = append(roots, home)
	}
	if _, err := os.Stat("/Applications"); err == nil {
		roots = append(roots, "/Applications")
	}
	if len(roots) == 0 {
		roots = append(roots, ".")
	}
	return roots
}

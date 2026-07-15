//go:build !darwin

// Non-macOS stub: searchfs(2) is a macOS-only syscall, so the fallback engine
// is unavailable off-platform. The authoritative build target is the
// macos-latest CI runner; this stub only exists so the package still compiles
// for local tooling on other OSes.
package searchfs

// Options controls a searchfs fallback query.
type Options struct {
	DirsOnly  bool
	FilesOnly bool
	Limit     int
}

// Result is a single fallback hit.
type Result struct {
	Path  string
	IsDir bool
}

// Search always returns nil off macOS.
func Search(match string, opt Options) []Result { return nil }

// Available reports that the fallback is unavailable off macOS.
func Available() bool { return false }

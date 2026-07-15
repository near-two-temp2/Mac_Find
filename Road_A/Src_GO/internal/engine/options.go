// Package engine wraps the macOS searchfs(2) syscall to perform index-free,
// real-time filename search across APFS/HFS+ volumes.
//
// This file holds the pure-Go, platform-independent types so that the package
// documentation and the option surface can be inspected on any OS. The actual
// syscall implementation lives in searchfs_darwin.go (cgo, darwin only) with a
// stub in searchfs_other.go for non-darwin builds.
package engine

// MatchKind selects whether files, directories, or both are returned.
type MatchKind int

const (
	// MatchAll returns both files and directories.
	MatchAll MatchKind = iota
	// MatchFilesOnly returns only files.
	MatchFilesOnly
	// MatchDirsOnly returns only directories.
	MatchDirsOnly
)

// Options controls a single searchfs run.
type Options struct {
	// Term is the filename substring to search for. Empty term is rejected.
	Term string
	// Kind restricts results to files, directories, or both.
	Kind MatchKind
	// CaseSensitive makes matching case sensitive. searchfs itself is always
	// case-insensitive for substring matches, so when this is set the engine
	// applies an extra post-filter on the basename.
	CaseSensitive bool
	// Limit caps the number of returned results across all volumes. 0 = no cap.
	Limit int
	// Volumes lists mount points to search. When empty the engine searches
	// "/" and, on Catalina+, "/System/Volumes/Data".
	Volumes []string
}

// Result is a single matched filesystem object.
type Result struct {
	Path string
	// IsDir is best-effort: searchfs returns the object id, not the type, so
	// this is derived from a lightweight lstat during path resolution. It may
	// be false if the object vanished between match and stat.
	IsDir bool
}

// DefaultVolumes returns the mount points searched when Options.Volumes is
// empty. On non-darwin builds it returns just "/".
func DefaultVolumes() []string {
	return defaultVolumes()
}

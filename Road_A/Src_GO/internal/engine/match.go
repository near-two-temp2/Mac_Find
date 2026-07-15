package engine

import (
	"path/filepath"
	"strings"
)

// maxResults is a hard ceiling on how many hits the engine buffers per volume.
// It bounds memory (each go_hit is PATH_MAX bytes) and keeps the GUI list
// responsive regardless of the user-supplied limit.
const maxResults = 100000

// caseSensitiveBasenameMatch reports whether term appears as a case-sensitive
// substring of the path's final component. searchfs matches case-insensitively,
// so this is the post-filter applied when Options.CaseSensitive is set.
func caseSensitiveBasenameMatch(path, term string) bool {
	return strings.Contains(filepath.Base(path), term)
}

package index

import (
	"io/fs"
	"path/filepath"
	"strings"
)

// defaultSkipDirs are directory names we never descend into. They mirror
// Cling's default ignore groups: caches, VCS internals and noisy build
// output that would bloat the index without helping a file search.
var defaultSkipDirs = map[string]bool{
	".git":          true,
	"node_modules":  true,
	".Trash":        true,
	"Caches":        true,
	".cache":        true,
	"DerivedData":   true,
	".build":        true,
	".gradle":       true,
	"Pods":          true,
	".Spotlight-V100": true,
	".fseventsd":    true,
	".DocumentRevisions-V100": true,
}

// BuildOptions controls a filesystem scan.
type BuildOptions struct {
	Roots       []string        // directories to walk (default: $HOME)
	SkipHidden  bool            // skip dot-files and dot-directories
	SkipDirs    map[string]bool // directory names to prune (nil = defaults)
	Progress    func(count int) // optional callback, invoked periodically
	progressEvery int           // internal: how often to report (default 5000)
}

// Build walks the roots in opts and returns a fully populated Index. Symlinks
// are not followed to avoid cycles and duplicate coverage. Permission errors
// on individual subtrees are skipped rather than aborting the whole scan.
func Build(opts BuildOptions) (*Index, error) {
	b := NewBuilder()
	roots := opts.Roots
	if len(roots) == 0 {
		roots = []string{"."}
	}
	skip := opts.SkipDirs
	if skip == nil {
		skip = defaultSkipDirs
	}
	every := opts.progressEvery
	if every == 0 {
		every = 5000
	}

	for _, root := range roots {
		root = filepath.Clean(root)
		_ = filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
			if err != nil {
				// Unreadable entry (permissions, races): skip its subtree but
				// keep the overall walk going.
				if d != nil && d.IsDir() {
					return fs.SkipDir
				}
				return nil
			}

			name := d.Name()
			isDir := d.IsDir()

			if path != root {
				if opts.SkipHidden && strings.HasPrefix(name, ".") {
					if isDir {
						return fs.SkipDir
					}
					return nil
				}
				if isDir && skip[name] {
					return fs.SkipDir
				}
			}

			// Don't follow symlinks: descending them risks cycles and double
			// counting. They're still recorded as leaf entries.
			if d.Type()&fs.ModeSymlink != 0 {
				b.Add(path, false)
				return nil
			}

			b.Add(path, isDir)
			if opts.Progress != nil && b.Len()%every == 0 {
				opts.Progress(b.Len())
			}
			return nil
		})
	}

	if opts.Progress != nil {
		opts.Progress(b.Len())
	}
	return b.Build(), nil
}

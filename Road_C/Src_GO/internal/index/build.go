package index

import (
	"encoding/binary"
	"io/fs"
	"os"
	"path/filepath"
	"strings"

	"macfind/roadc/internal/bitmask"
)

// entry is the transient record collected during a build, before serialization
// into the parallel arrays.
type entry struct {
	mask    uint64
	bnMask  uint64
	offset  uint32
	length  uint32
	bnStart uint32
	isDir   bool
}

// Build walks each root, collecting file/dir paths, and writes a binary index
// to outPath. It returns the number of entries indexed. Unreadable directories
// are skipped rather than aborting the whole walk.
//
// Network / FUSE / remote volumes are pruned before any stat or readdir touches
// them (see walkfilter.go): indexing them is slow, can hang, and — on this
// machine's rclone→B2 mounts — costs money. The prune is driven by the live
// mount table plus a static exclusion list, so it holds even if a root happens
// to point at (or cross into) a remote volume.
//
// There is no entry-count cap: coverage is bounded only by the roots, so the
// whole local filesystem can be indexed without silently dropping files
// (SEARCH_TEST_BASELINE.md #2).
func Build(roots []string, outPath string) (int, error) {
	var entries []entry
	var blob []byte

	add := func(path string, isDir bool) {
		lower := strings.ToLower(path)
		off := uint32(len(blob))
		blob = append(blob, lower...)
		bnStart := strings.LastIndexByte(lower, '/') + 1
		entries = append(entries, entry{
			mask:    bitmask.Of(lower),
			bnMask:  bitmask.Of(lower[bnStart:]),
			offset:  off,
			length:  uint32(len(lower)),
			bnStart: uint32(bnStart),
			isDir:   isDir,
		})
	}

	skip := newSkipper()
	for _, root := range roots {
		// Guard the root itself: never descend into a root that is (or lives
		// under) a network mount.
		if skip.shouldSkip(root) {
			continue
		}
		_ = filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
			if err != nil {
				// Skip unreadable entries (permission denied, races) and keep going.
				if d != nil && d.IsDir() {
					return fs.SkipDir
				}
				return nil
			}
			// Prune network / excluded directories before touching their contents.
			if d.IsDir() && skip.shouldSkip(path) {
				return fs.SkipDir
			}
			add(path, d.IsDir())
			return nil
		})
	}

	if err := serialize(entries, blob, outPath); err != nil {
		return 0, err
	}
	return len(entries), nil
}

// serialize writes the header, parallel arrays and blob to outPath atomically
// (temp file + rename) so a crash mid-write never leaves a half-index that
// would masquerade as valid.
func serialize(entries []entry, blob []byte, outPath string) error {
	n := len(entries)
	total := headerSize + n*entryStride + len(blob)
	buf := make([]byte, total)

	putHeader(buf, uint32(n), uint64(len(blob)))

	p := headerSize
	for i := range entries {
		e := &entries[i]
		binary.LittleEndian.PutUint64(buf[p:], e.mask)
		binary.LittleEndian.PutUint64(buf[p+8:], e.bnMask)
		binary.LittleEndian.PutUint32(buf[p+16:], e.offset)
		binary.LittleEndian.PutUint32(buf[p+20:], e.length)
		binary.LittleEndian.PutUint32(buf[p+24:], e.bnStart)
		if e.isDir {
			buf[p+28] = 1
		}
		p += entryStride
	}
	copy(buf[p:], blob)

	if dir := filepath.Dir(outPath); dir != "" {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			return err
		}
	}
	tmp := outPath + ".tmp"
	if err := os.WriteFile(tmp, buf, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, outPath)
}

// DefaultRoots returns the default scan set: the user's home directory,
// /Applications, and every *local* volume mounted under /Volumes. Adding the
// local /Volumes entries widens coverage to secondary disks (e.g. the baseline's
// /Volumes/MacDisk/Users/Shared/temp_test) while network mounts there are pruned
// by the walk's skipper — so we never index a remote volume. Falls back to "/"
// if HOME is unset.
func DefaultRoots() []string {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		return []string{"/"}
	}
	roots := []string{home, "/Applications"}
	roots = append(roots, localVolumeRoots(home)...)
	return roots
}

// localVolumeRoots lists local volumes mounted under /Volumes worth indexing,
// excluding network mounts and the home volume (already covered by `home`).
// It reads the live mount table; if that is unavailable it returns nothing
// (home + /Applications still gets indexed).
func localVolumeRoots(home string) []string {
	var out []string
	seen := map[string]bool{}
	for _, m := range listMounts() {
		if !strings.HasPrefix(m.point, "/Volumes/") {
			continue
		}
		if !m.isLocal || !localFSTypes[strings.ToLower(m.fstype)] {
			continue // network / FUSE volume — skip
		}
		// The home directory's volume is already a root; don't double-index it.
		if strings.HasPrefix(home, m.point+"/") || home == m.point {
			continue
		}
		if !seen[m.point] {
			seen[m.point] = true
			out = append(out, m.point)
		}
	}
	return out
}

// DefaultPath is where the index is cached, mirroring Cling's location scheme.
func DefaultPath() string {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		home = os.TempDir()
	}
	return filepath.Join(home, "Library", "Caches", "macfind-roadc-go", "index.idx")
}

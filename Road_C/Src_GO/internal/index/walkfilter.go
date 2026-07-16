package index

import (
	"os"
	"path/filepath"
	"strings"
)

// localFSTypes is the allowlist of filesystem types we are willing to index.
// Everything else (macfuse, nfs, smbfs, afpfs, cifs, webdav, FileProvider, …)
// is treated as a network / remote volume and pruned. See SEARCH_TEST_BASELINE.md.
var localFSTypes = map[string]bool{
	"apfs": true,
	"hfs":  true,
}

// staticExcludePrefixes are paths we always prune regardless of what the mount
// table reports, as a belt-and-suspenders guard against the known rclone→B2 and
// cloud-storage mounts on this machine (project CLAUDE.md / SEARCH_TEST_BASELINE.md).
// h2- covers h2-bu-01, h2_bu_01_b2, h2_open_rsh, etc.
func staticExcludePrefixes() []string {
	prefixes := []string{
		"/Volumes/Disk/h2-",
		"/Volumes/Disk/h2_",
	}
	if home, err := os.UserHomeDir(); err == nil && home != "" {
		prefixes = append(prefixes,
			filepath.Join(home, "Library", "CloudStorage"),
		)
	}
	return prefixes
}

// skipper decides, during the index walk, whether a directory must be pruned
// because it is a network / FUSE / remote volume (or a known bad mount). It is
// built once per Build call from the live mount table plus the static excludes.
type skipper struct {
	// networkPoints are mount points whose filesystem type is not in the local
	// allowlist (or that lack MNT_LOCAL). Walking into these is forbidden.
	networkPoints []string
	// staticPrefixes are always-excluded path prefixes.
	staticPrefixes []string
}

// newSkipper reads the mount table and precomputes the set of forbidden paths.
func newSkipper() *skipper {
	s := &skipper{staticPrefixes: staticExcludePrefixes()}
	for _, m := range listMounts() {
		// A mount is "network/remote" if it is not flagged local, or its fstype
		// is outside our apfs/hfs allowlist. The root "/" and the data volume are
		// apfs+local, so they pass; rclone/macfuse/nfs/smb points do not.
		if !m.isLocal || !localFSTypes[strings.ToLower(m.fstype)] {
			// "/" would never be a network mount, but guard anyway so we never
			// prune the entire filesystem by accident.
			if m.point != "" && m.point != "/" {
				s.networkPoints = append(s.networkPoints, m.point)
			}
		}
	}
	return s
}

// shouldSkip reports whether the directory at path must not be descended into.
// It matches both exact network mount points and any known-bad prefix, so a
// path that lands on (or underneath) a remote volume is pruned before the walk
// can issue any stat/readdir against it.
func (s *skipper) shouldSkip(path string) bool {
	for _, p := range s.staticPrefixes {
		if path == p || strings.HasPrefix(path, p) {
			return true
		}
	}
	for _, mp := range s.networkPoints {
		if path == mp || strings.HasPrefix(path, mp+string(filepath.Separator)) {
			return true
		}
	}
	return false
}

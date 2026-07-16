//go:build darwin

// This file provides the macOS mount-table inspection used to keep index
// builds off network / FUSE volumes. Indexing a network drive is slow, can
// hang, and (critically on this machine) triggers Backblaze B2 API charges via
// the rclone→B2 mounts. So the walk must stay on local APFS/HFS volumes only.
//
// See ../../SEARCH_TEST_BASELINE.md "索引构建硬性要求：避开所有网络驱动器".
package index

/*
#include <sys/param.h>
#include <sys/ucred.h>
#include <sys/mount.h>
#include <stdlib.h>
#include <string.h>

// go_getmounts fills `out` with the currently-mounted filesystems as
// newline-separated "<isLocal>\t<fstype>\t<mountpoint>" records, where isLocal
// is '1' when the MNT_LOCAL flag is set and '0' otherwise. Returns the number of
// bytes written, or -1 if the buffer was too small. Using getmntinfo (rather
// than parsing `mount` output) keeps this dependency-free and reads the same
// f_flags / f_fstypename the kernel reports.
static int go_getmounts(char *out, int outcap) {
    struct statfs *mnts = NULL;
    int n = getmntinfo(&mnts, MNT_NOWAIT);
    if (n <= 0) return 0;
    int written = 0;
    for (int i = 0; i < n; i++) {
        char local = (mnts[i].f_flags & MNT_LOCAL) ? '1' : '0';
        const char *fst = mnts[i].f_fstypename;
        const char *mp  = mnts[i].f_mntonname;
        int need = 1 /*local*/ + 1 /*tab*/ + (int)strlen(fst) + 1 /*tab*/
                 + (int)strlen(mp) + 1 /*newline*/;
        if (written + need >= outcap) return -1;
        out[written++] = local;
        out[written++] = '\t';
        memcpy(out + written, fst, strlen(fst)); written += (int)strlen(fst);
        out[written++] = '\t';
        memcpy(out + written, mp, strlen(mp));  written += (int)strlen(mp);
        out[written++] = '\n';
    }
    return written;
}
*/
import "C"

import (
	"strings"
	"unsafe"
)

// mount is one entry from the kernel mount table.
type mount struct {
	point   string
	fstype  string
	isLocal bool
}

// listMounts returns the current mount table via getmntinfo(3). On any failure
// it returns nil, and callers fall back to the static exclusion list — so a
// build never accidentally indexes a network volume just because this probe
// came up empty.
func listMounts() []mount {
	const cap = 256 << 10 // 256 KiB is plenty for any realistic mount table
	buf := make([]byte, cap)
	n := C.go_getmounts((*C.char)(unsafe.Pointer(&buf[0])), C.int(cap))
	if n <= 0 {
		return nil
	}
	var out []mount
	for _, line := range strings.Split(string(buf[:n]), "\n") {
		if line == "" {
			continue
		}
		parts := strings.SplitN(line, "\t", 3)
		if len(parts) != 3 {
			continue
		}
		out = append(out, mount{
			point:   parts[2],
			fstype:  parts[1],
			isLocal: parts[0] == "1",
		})
	}
	return out
}

//go:build darwin

// Package searchfs is the real-time fallback engine: a cgo wrapper around the
// macOS searchfs(2) syscall that searches the APFS/HFS+ catalog B-tree directly
// (see Open_Ref/searchfs/main.m). It is used when the binary index is missing
// or corrupt, guaranteeing 100% accurate results at the cost of higher latency.
package searchfs

/*
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <sys/attr.h>
#include <sys/param.h>
#include <sys/vnode.h>
#include <sys/fsgetpath.h>
#include <sys/mount.h>

// searchfs(2) has no public header prototype in the SDK (the reference pulls it
// in transitively via Foundation). Declare it explicitly so cgo links against
// the libSystem symbol without an implicit-declaration error under -Werror.
int searchfs(const char *path, struct fssearchblock *searchBlock,
             unsigned long *nummatches, unsigned int scriptcode,
             unsigned int options, struct searchstate *state);

// Mirrors the packed structures from the searchfs reference implementation.
struct packed_name_attr {
    u_int32_t            size;
    struct attrreference ref;
    char                 name[PATH_MAX];
};

struct packed_attr_ref {
    u_int32_t            size;
    struct attrreference ref;
};

struct packed_result {
    u_int32_t       size;
    struct fsid     fs_id;
    struct fsobj_id obj_id;
};

#define MAX_MATCHES       32
#define MAX_EBUSY_RETRIES 5

// go_searchfs runs a full searchfs() scan of one volume for `match`, writing up
// to `limit` newline-separated matching paths into `out` (a Go-owned buffer of
// `outcap` bytes). It returns the number of bytes written, or -1 on a hard
// error. Substring matching is performed by the kernel (SRCHFS_MATCHPARTIALNAMES);
// path formatting is done here to keep the Go side allocation-light.
static int go_searchfs(const char *volpath, const char *match,
                       int dirs_only, int files_only, int limit,
                       char *out, int outcap) {
    int  err = 0;
    long matches;
    unsigned int search_options;
    struct fssearchblock search_blk;
    struct attrlist      return_list;
    struct searchstate   search_state;
    struct packed_name_attr info1;
    struct packed_attr_ref  info2;
    struct packed_result result_buffer[MAX_MATCHES];
    int  ebusy_count = 0;
    int  written = 0;
    int  count = 0;

catalog_changed:
    memset(&search_blk, 0, sizeof(search_blk));
    search_blk.searchattrs.bitmapcount = ATTR_BIT_MAP_COUNT;
    search_blk.searchattrs.commonattr  = ATTR_CMN_NAME;

    return_list.bitmapcount = ATTR_BIT_MAP_COUNT;
    return_list.reserved    = 0;
    return_list.commonattr  = ATTR_CMN_FSID | ATTR_CMN_OBJID;
    return_list.volattr = return_list.dirattr = 0;
    return_list.fileattr = return_list.forkattr = 0;

    search_blk.returnattrs     = &return_list;
    search_blk.returnbuffer    = result_buffer;
    search_blk.returnbuffersize = sizeof(result_buffer);

    strlcpy(info1.name, match, sizeof(info1.name));
    info1.ref.attr_dataoffset = sizeof(struct attrreference);
    info1.ref.attr_length     = (u_int32_t)strlen(info1.name) + 1;
    info1.size = sizeof(struct attrreference) + info1.ref.attr_length;
    search_blk.searchparams1     = &info1;
    search_blk.sizeofsearchparams1 = info1.size + sizeof(u_int32_t);

    info2.size = sizeof(struct attrreference);
    info2.ref.attr_dataoffset = sizeof(struct attrreference);
    info2.ref.attr_length     = 0;
    search_blk.searchparams2     = &info2;
    search_blk.sizeofsearchparams2 = sizeof(info2);

    search_blk.maxmatches      = MAX_MATCHES;
    search_blk.timelimit.tv_sec  = 1;
    search_blk.timelimit.tv_usec = 0;

    search_options = SRCHFS_START | SRCHFS_MATCHPARTIALNAMES;
    if (!dirs_only)  search_options |= SRCHFS_MATCHFILES;
    if (!files_only) search_options |= SRCHFS_MATCHDIRS;

    do {
        err = searchfs(volpath, &search_blk, (unsigned long *)&matches, 0,
                       search_options, &search_state);
        if (err == -1) err = errno;

        if ((err == 0 || err == EAGAIN) && matches > 0) {
            char *ptr     = (char *)&result_buffer[0];
            char *end_ptr = ptr + sizeof(result_buffer);
            for (long i = 0; i < matches; ++i) {
                struct packed_result *r = (struct packed_result *)ptr;
                char path_buf[PATH_MAX];
                ssize_t sz = fsgetpath(path_buf, sizeof(path_buf), &r->fs_id,
                                       (uint64_t)r->obj_id.fid_objno |
                                       ((uint64_t)r->obj_id.fid_generation << 32));
                if (sz > -1) {
                    int need = (int)sz + 1;
                    if (written + need < outcap) {
                        memcpy(out + written, path_buf, sz);
                        written += sz;
                        out[written++] = '\n';
                        if (++count >= limit) return written;
                    } else {
                        return written; // buffer full
                    }
                }
                ptr += r->size;
                if (ptr > end_ptr) break;
            }
        }

        if (err == EBUSY && ebusy_count++ < MAX_EBUSY_RETRIES) {
            goto catalog_changed;
        }
        if (err != 0 && err != EAGAIN) {
            return written > 0 ? written : -1;
        }
        search_options &= ~SRCHFS_START;
    } while (err == EAGAIN);

    return written;
}
*/
import "C"

import (
	"strings"
	"unsafe"
)

// defaultVolumes mirror the reference's Catalina+ dual-volume strategy.
var defaultVolumes = []string{"/", "/System/Volumes/Data"}

// Options controls a searchfs fallback query.
type Options struct {
	DirsOnly  bool
	FilesOnly bool
	Limit     int // 0 => unlimited (capped by buffer)
}

// Result is a single fallback hit. Score is unused here (kept 0) so it composes
// with the index Match ordering.
type Result struct {
	Path  string
	IsDir bool
}

// Search runs a real-time searchfs() scan across the default system/data
// volumes for the substring `match`, de-duplicating paths. It never returns an
// error to the caller: on syscall failure it simply yields fewer/zero results,
// so the UI degrades gracefully.
func Search(match string, opt Options) []Result {
	match = strings.TrimSpace(match)
	if match == "" {
		return nil
	}
	limit := opt.Limit
	if limit <= 0 {
		limit = 5000
	}

	const bufCap = 4 << 20 // 4 MiB scratch for newline-joined paths
	buf := make([]byte, bufCap)

	cMatch := C.CString(match)
	defer C.free(unsafe.Pointer(cMatch))

	seen := make(map[string]struct{})
	var out []Result

	for _, vol := range defaultVolumes {
		if len(out) >= limit {
			break
		}
		cVol := C.CString(vol)
		n := C.go_searchfs(cVol, cMatch,
			boolToInt(opt.DirsOnly), boolToInt(opt.FilesOnly),
			C.int(limit), (*C.char)(unsafe.Pointer(&buf[0])), C.int(bufCap))
		C.free(unsafe.Pointer(cVol))
		if n <= 0 {
			continue
		}
		for _, p := range strings.Split(string(buf[:n]), "\n") {
			if p == "" {
				continue
			}
			if _, dup := seen[p]; dup {
				continue
			}
			seen[p] = struct{}{}
			out = append(out, Result{Path: p, IsDir: false})
			if len(out) >= limit {
				break
			}
		}
	}
	return out
}

// Available reports whether the searchfs fallback is usable on this platform.
func Available() bool { return true }

func boolToInt(b bool) C.int {
	if b {
		return 1
	}
	return 0
}

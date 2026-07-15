//go:build darwin

package engine

/*
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <sys/attr.h>
#include <sys/param.h>
#include <sys/vnode.h>
#include <sys/fsgetpath.h>
#include <sys/mount.h>
#include <sys/stat.h>

// Packed structures mirror Open_Ref/searchfs/main.m. searchfs() returns a
// tightly packed buffer that must be walked by each record's leading size
// field, so the layout has to match the kernel's exactly.

struct packed_name_attr {
    u_int32_t            size;               // Of the remaining fields
    struct attrreference ref;                // Offset/length of name itself
    char                 name[PATH_MAX];
};

struct packed_attr_ref {
    u_int32_t            size;               // Of the remaining fields
    struct attrreference ref;                // Offset/length of attr itself
};

struct packed_result {
    u_int32_t       size;                    // Including size field itself
    struct fsid     fs_id;
    struct fsobj_id obj_id;
};

#define GO_MAX_MATCHES       64
#define GO_MAX_EBUSY_RETRIES 5

// go_hit is one resolved match handed back to Go. We resolve the path (and a
// best-effort directory flag) inside C so the cgo boundary is crossed once per
// syscall batch rather than once per record.
struct go_hit {
    char path[PATH_MAX];
    int  is_dir;
};

// searchfs_run performs a full searchfs() sweep of one volume.
//
//   volpath      mount point to search ("/", "/System/Volumes/Data", ...)
//   term         filename substring (must be non-empty)
//   dirs_only    1 => match directories only
//   files_only   1 => match files only
//   limit        stop after this many hits (0 = unlimited)
//   out          caller-allocated array of `cap` go_hit entries
//   cap          capacity of `out`
//
// Returns the number of hits written to `out`, or a negative errno on a fatal
// searchfs error (EBUSY retries and EAGAIN continuation are handled here).
static int searchfs_run(const char *volpath,
                        const char *term,
                        int dirs_only,
                        int files_only,
                        int limit,
                        struct go_hit *out,
                        int cap) {
    int                     err = 0;
    unsigned long           matches;
    unsigned int            search_options;
    struct fssearchblock    search_blk;
    struct attrlist         return_list;
    struct searchstate      search_state;
    struct packed_name_attr info1;
    struct packed_attr_ref  info2;
    struct packed_result    result_buffer[GO_MAX_MATCHES];
    int                     ebusy_count = 0;
    int                     hit_count = 0;

catalog_changed:
    memset(&search_blk, 0, sizeof(search_blk));
    search_blk.searchattrs.bitmapcount = ATTR_BIT_MAP_COUNT;
    search_blk.searchattrs.commonattr  = ATTR_CMN_NAME;

    return_list.bitmapcount = ATTR_BIT_MAP_COUNT;
    return_list.reserved    = 0;
    return_list.commonattr  = ATTR_CMN_FSID | ATTR_CMN_OBJID;
    return_list.volattr     = 0;
    return_list.dirattr     = 0;
    return_list.fileattr    = 0;
    return_list.forkattr    = 0;

    search_blk.returnattrs     = &return_list;
    search_blk.returnbuffer    = result_buffer;
    search_blk.returnbuffersize = sizeof(result_buffer);

    // searchparams1 carries the name to match.
    strlcpy(info1.name, term, sizeof(info1.name));
    info1.ref.attr_dataoffset = sizeof(struct attrreference);
    info1.ref.attr_length     = (u_int32_t)strlen(info1.name) + 1;
    info1.size                = sizeof(struct attrreference) + info1.ref.attr_length;
    search_blk.searchparams1       = &info1;
    search_blk.sizeofsearchparams1 = info1.size + sizeof(u_int32_t);

    // searchparams2 is an empty upper-bound attr ref.
    info2.size                = sizeof(struct attrreference);
    info2.ref.attr_dataoffset = sizeof(struct attrreference);
    info2.ref.attr_length     = 0;
    search_blk.searchparams2       = &info2;
    search_blk.sizeofsearchparams2 = sizeof(info2);

    search_blk.maxmatches      = GO_MAX_MATCHES;
    search_blk.timelimit.tv_sec  = 1;
    search_blk.timelimit.tv_usec = 0;

    search_options = SRCHFS_START | SRCHFS_MATCHPARTIALNAMES;
    if (!dirs_only) {
        search_options |= SRCHFS_MATCHFILES;
    }
    if (!files_only) {
        search_options |= SRCHFS_MATCHDIRS;
    }

    do {
        err = searchfs(volpath, &search_blk, &matches, 0, search_options, &search_state);
        if (err == -1) {
            err = errno;
        }

        if ((err == 0 || err == EAGAIN) && matches > 0) {
            char *ptr     = (char *)&result_buffer[0];
            char *end_ptr = ptr + sizeof(result_buffer);

            for (unsigned long i = 0; i < matches; ++i) {
                struct packed_result *result_p = (struct packed_result *)ptr;

                char path_buf[PATH_MAX];
                ssize_t sz = fsgetpath((char *)&path_buf,
                                       sizeof(path_buf),
                                       &result_p->fs_id,
                                       (uint64_t)result_p->obj_id.fid_objno |
                                       ((uint64_t)result_p->obj_id.fid_generation << 32));
                if (sz > -1) {
                    if (hit_count < cap) {
                        strlcpy(out[hit_count].path, path_buf, PATH_MAX);
                        struct stat st;
                        out[hit_count].is_dir =
                            (lstat(path_buf, &st) == 0 && S_ISDIR(st.st_mode)) ? 1 : 0;
                        hit_count++;
                    }
                    if (limit && hit_count >= limit) {
                        return hit_count;
                    }
                    if (hit_count >= cap) {
                        return hit_count;
                    }
                }
                // fsgetpath failing usually means the object was deleted
                // between match and lookup; skip silently.

                ptr += result_p->size;
                if (ptr > end_ptr) {
                    break;
                }
            }
        }

        // EBUSY => the catalog changed mid-search; restart a few times.
        if (err == EBUSY && ebusy_count++ < GO_MAX_EBUSY_RETRIES) {
            goto catalog_changed;
        }

        if (err != 0 && err != EAGAIN) {
            return -err;
        }

        search_options &= ~SRCHFS_START;
    } while (err == EAGAIN);

    return hit_count;
}

// vol_supports_searchfs mirrors the capability probe in main.m: a volume must
// advertise VOL_CAP_INT_SEARCHFS to be searchable.
static int vol_supports_searchfs(const char *path) {
    struct vol_attr_buf {
        u_int32_t               size;
        vol_capabilities_attr_t vol_capabilities;
    } __attribute__((aligned(4), packed));

    struct attrlist attrList;
    memset(&attrList, 0, sizeof(attrList));
    attrList.bitmapcount = ATTR_BIT_MAP_COUNT;
    attrList.volattr     = (ATTR_VOL_INFO | ATTR_VOL_CAPABILITIES);

    struct vol_attr_buf attrBuf;
    memset(&attrBuf, 0, sizeof(attrBuf));

    if (getattrlist(path, &attrList, &attrBuf, sizeof(attrBuf), 0) != 0) {
        return 0;
    }
    if (attrBuf.size != sizeof(attrBuf)) {
        return 0;
    }
    if ((attrBuf.vol_capabilities.valid[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS) &&
        (attrBuf.vol_capabilities.capabilities[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS)) {
        return 1;
    }
    return 0;
}
*/
import "C"

import (
	"fmt"
	"os"
	"strings"
	"unsafe"
)

const dataVolume = "/System/Volumes/Data"

// defaultVolumes returns "/" plus the Catalina+ data volume when present and
// searchfs-capable.
func defaultVolumes() []string {
	vols := []string{"/"}
	if _, err := os.Stat(dataVolume); err == nil && volSupportsSearchfs(dataVolume) {
		vols = append(vols, dataVolume)
	}
	return vols
}

// volSupportsSearchfs reports whether a mount point advertises searchfs.
func volSupportsSearchfs(path string) bool {
	cpath := C.CString(path)
	defer C.free(unsafe.Pointer(cpath))
	return C.vol_supports_searchfs(cpath) == 1
}

// Search runs searchfs across the requested (or default) volumes and returns
// the matched results. It is the concrete darwin implementation behind the
// exported Search wrapper.
func Search(opts Options) ([]Result, error) {
	if strings.TrimSpace(opts.Term) == "" {
		return nil, fmt.Errorf("empty search term")
	}

	vols := opts.Volumes
	if len(vols) == 0 {
		vols = defaultVolumes()
	}

	dirsOnly := 0
	filesOnly := 0
	switch opts.Kind {
	case MatchFilesOnly:
		filesOnly = 1
	case MatchDirsOnly:
		dirsOnly = 1
	}

	// The kernel matches case-insensitively; when the caller wants case
	// sensitivity we over-fetch and post-filter, so ask the kernel for more.
	bufCap := opts.Limit
	if bufCap == 0 || opts.CaseSensitive {
		bufCap = maxResults
	}
	if bufCap > maxResults {
		bufCap = maxResults
	}

	cterm := C.CString(opts.Term)
	defer C.free(unsafe.Pointer(cterm))

	buf := make([]C.struct_go_hit, bufCap)

	var results []Result
	var lastErr error
	seen := make(map[string]struct{})

	for _, vol := range vols {
		if !volSupportsSearchfs(vol) {
			continue
		}
		cvol := C.CString(vol)
		n := C.searchfs_run(
			cvol,
			cterm,
			C.int(dirsOnly),
			C.int(filesOnly),
			C.int(bufCap),
			(*C.struct_go_hit)(unsafe.Pointer(&buf[0])),
			C.int(bufCap),
		)
		C.free(unsafe.Pointer(cvol))

		if n < 0 {
			// searchfs failed on this volume (commonly EPERM when the process
			// lacks Full Disk Access). Remember it but keep trying the other
			// volumes, mirroring main.m's non-fatal, keep-going behavior.
			lastErr = fmt.Errorf("searchfs(%s) failed: errno %d", vol, -int(n))
			continue
		}

		for i := 0; i < int(n); i++ {
			path := C.GoString(&buf[i].path[0])
			if opts.CaseSensitive && !caseSensitiveBasenameMatch(path, opts.Term) {
				continue
			}
			if _, dup := seen[path]; dup {
				continue
			}
			seen[path] = struct{}{}
			results = append(results, Result{
				Path:  path,
				IsDir: buf[i].is_dir == 1,
			})
			if opts.Limit > 0 && len(results) >= opts.Limit {
				return results, nil
			}
		}
	}

	// Only surface an error when nothing at all was found; a partial success
	// (some volume returned hits) is reported as success.
	if len(results) == 0 && lastErr != nil {
		return results, lastErr
	}
	return results, nil
}

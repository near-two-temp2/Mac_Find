/*
 * csearchfs.h — C shim exposing macOS searchfs() to Swift.
 *
 * The delicate packed-attribute pointer arithmetic around searchfs()/fsgetpath()
 * is kept in C (mirroring Open_Ref/searchfs/main.m, BSD-3-Clause) rather than
 * reimplemented in Swift, because the layout of `struct fssearchblock` and the
 * ATTR_* return buffer is easiest to get right against the system headers.
 *
 * The whole search loop lives in csfs_search(); it invokes a Swift-provided
 * callback once per matched path so Swift can filter/collect results.
 */

#ifndef CSEARCHFS_H
#define CSEARCHFS_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Match-scope options, combined bitwise into csfs_options_t. */
typedef unsigned int csfs_options_t;

enum {
    CSFS_MATCH_FILES   = 1u << 0,  /* include files      (SRCHFS_MATCHFILES) */
    CSFS_MATCH_DIRS    = 1u << 1,  /* include dirs       (SRCHFS_MATCHDIRS)  */
    CSFS_PARTIAL       = 1u << 2,  /* substring match    (SRCHFS_MATCHPARTIALNAMES) */
    CSFS_SKIP_PACKAGES = 1u << 3,  /* SRCHFS_SKIPPACKAGES  */
    CSFS_SKIP_INVISIBLE= 1u << 4   /* SRCHFS_SKIPINVISIBLE */
};

/*
 * Result callback. Invoked for every filesystem object searchfs() returns whose
 * path could be resolved. Returning 0 asks csfs_search to stop early (limit hit,
 * cancellation); returning non-zero continues.
 *
 * `path`    NUL-terminated absolute path (valid only for the callback duration).
 * `context` opaque pointer forwarded from csfs_search.
 */
typedef int (*csfs_result_cb)(const char *path, void *context);

/*
 * Run a searchfs() scan on one volume.
 *
 *   volume_path   mount point to scan (e.g. "/" or "/System/Volumes/Data").
 *   search_term   filename fragment to match (case-insensitive at kernel level).
 *   options       bitwise OR of CSFS_* flags.
 *   cb            per-result callback (may be NULL to just count).
 *   context       forwarded to cb.
 *
 * Returns the number of results delivered to the callback, or a negative errno
 * (as -errno) on a fatal searchfs() error.
 */
long csfs_search(const char *volume_path,
                 const char *search_term,
                 csfs_options_t options,
                 csfs_result_cb cb,
                 void *context);

/* Return 1 if the volume at `path` supports catalog search (VOL_CAP_INT_SEARCHFS). */
int csfs_volume_supports_searchfs(const char *path);

/* Return 1 if /System/Volumes/Data exists and supports catalog search. */
int csfs_data_volume_available(void);

#ifdef __cplusplus
}
#endif

#endif /* CSEARCHFS_H */

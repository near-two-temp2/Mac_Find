/*
 * csearchfs.c — searchfs() driver, adapted from Open_Ref/searchfs/main.m
 * (Copyright (c) 2017-2025 Sveinbjorn Thordarson, BSD-3-Clause).
 *
 * In Road_C this is the *fallback* engine: the Swift layer prefers the mmap
 * binary index and only calls here when the index is unusable. The per-result
 * output/filtering of the original is replaced by a callback so Swift owns
 * collection, limits, cancellation and higher-level filters.
 */

#include "csearchfs.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <sys/attr.h>
#include <sys/param.h>
#include <sys/vnode.h>
#include <sys/fsgetpath.h>
#include <sys/mount.h>

#define CSFS_MAX_MATCHES        64
#define CSFS_MAX_EBUSY_RETRIES  5
#define CSFS_DATA_VOLUME        "/System/Volumes/Data"

/* Packed request/return structures — layout matches the reference main.m. */
struct packed_name_attr {
    u_int32_t            size;                  /* of the remaining fields */
    struct attrreference ref;                   /* offset/length of name itself */
    char                 name[PATH_MAX];
};

struct packed_attr_ref {
    u_int32_t            size;
    struct attrreference ref;
};

struct packed_result {
    u_int32_t       size;                       /* including this size field */
    struct fsid     fs_id;
    struct fsobj_id obj_id;
};
typedef struct packed_result packed_result;

long csfs_search(const char *volume_path,
                 const char *search_term,
                 csfs_options_t options,
                 csfs_result_cb cb,
                 void *context) {
    if (volume_path == NULL || search_term == NULL) {
        return -EINVAL;
    }
    if (strlen(search_term) >= PATH_MAX) {
        return -ENAMETOOLONG;
    }

    int                    err = 0;
    int                    ebusy_count = 0;
    unsigned long          matches = 0;
    unsigned int           search_options;
    struct fssearchblock   search_blk;
    struct attrlist        return_list;
    struct searchstate     search_state;
    struct packed_name_attr info1;
    struct packed_attr_ref  info2;
    packed_result          result_buffer[CSFS_MAX_MATCHES];
    long                   delivered = 0;

catalog_changed:
    /* Attributes to search on: only the common name. */
    search_blk.searchattrs.bitmapcount = ATTR_BIT_MAP_COUNT;
    search_blk.searchattrs.reserved    = 0;
    search_blk.searchattrs.commonattr  = ATTR_CMN_NAME;
    search_blk.searchattrs.volattr     = 0;
    search_blk.searchattrs.dirattr     = 0;
    search_blk.searchattrs.fileattr    = 0;
    search_blk.searchattrs.forkattr    = 0;

    /* Attributes to return for each match: fsid + objid, so fsgetpath() works. */
    search_blk.returnattrs = &return_list;
    return_list.bitmapcount = ATTR_BIT_MAP_COUNT;
    return_list.reserved    = 0;
    return_list.commonattr  = ATTR_CMN_FSID | ATTR_CMN_OBJID;
    return_list.volattr     = 0;
    return_list.dirattr     = 0;
    return_list.fileattr    = 0;
    return_list.forkattr    = 0;

    search_blk.returnbuffer     = result_buffer;
    search_blk.returnbuffersize = sizeof(result_buffer);

    /* searchparams1 carries the name to match. */
    strlcpy(info1.name, search_term, sizeof(info1.name));
    info1.ref.attr_dataoffset = sizeof(struct attrreference);
    info1.ref.attr_length     = (u_int32_t)strlen(info1.name) + 1;
    info1.size                = sizeof(struct attrreference) + info1.ref.attr_length;
    search_blk.searchparams1     = &info1;
    search_blk.sizeofsearchparams1 = info1.size + sizeof(u_int32_t);

    /* searchparams2 is unused but must be well-formed. */
    info2.size                = sizeof(struct attrreference);
    info2.ref.attr_dataoffset = sizeof(struct attrreference);
    info2.ref.attr_length     = 0;
    search_blk.searchparams2     = &info2;
    search_blk.sizeofsearchparams2 = sizeof(info2);

    search_blk.maxmatches      = CSFS_MAX_MATCHES;
    search_blk.timelimit.tv_sec  = 1;
    search_blk.timelimit.tv_usec = 0;

    /* Translate high-level options into SRCHFS_* flags. */
    search_options = SRCHFS_START;
    if (options & CSFS_MATCH_FILES)    search_options |= SRCHFS_MATCHFILES;
    if (options & CSFS_MATCH_DIRS)     search_options |= SRCHFS_MATCHDIRS;
    if (options & CSFS_PARTIAL)        search_options |= SRCHFS_MATCHPARTIALNAMES;
    if (options & CSFS_SKIP_PACKAGES)  search_options |= SRCHFS_SKIPPACKAGES;
    if (options & CSFS_SKIP_INVISIBLE) search_options |= SRCHFS_SKIPINVISIBLE;

    do {
        err = searchfs(volume_path, &search_blk, &matches, 0, search_options, &search_state);
        if (err == -1) {
            err = errno;
        }

        if ((err == 0 || err == EAGAIN) && matches > 0) {
            char *ptr     = (char *)&result_buffer[0];
            char *end_ptr = ptr + sizeof(result_buffer);

            for (unsigned long i = 0; i < matches; ++i) {
                packed_result *result_p = (packed_result *)ptr;

                char path_buf[PATH_MAX];
                ssize_t size = fsgetpath((char *)&path_buf,
                                         sizeof(path_buf),
                                         &result_p->fs_id,
                                         (uint64_t)result_p->obj_id.fid_objno |
                                         ((uint64_t)result_p->obj_id.fid_generation << 32));
                if (size > -1) {
                    delivered++;
                    if (cb != NULL) {
                        if (cb(path_buf, context) == 0) {
                            /* Callback asked us to stop (limit / cancel). */
                            return delivered;
                        }
                    }
                }
                /* else: object vanished between match and path lookup — skip silently. */

                ptr += result_p->size;
                if (ptr > end_ptr) {
                    break;
                }
            }
        }

        /* EBUSY means the catalog changed mid-search: restart a few times. */
        if (err == EBUSY && ebusy_count++ < CSFS_MAX_EBUSY_RETRIES) {
            goto catalog_changed;
        }

        if (err != 0 && err != EAGAIN && err != EBUSY) {
            return -err;
        }

        search_options &= ~SRCHFS_START;
    } while (err == EAGAIN);

    return delivered;
}

int csfs_volume_supports_searchfs(const char *path) {
    if (path == NULL) {
        return 0;
    }

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

    if ((attrBuf.vol_capabilities.valid[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS &&
        (attrBuf.vol_capabilities.capabilities[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS) {
        return 1;
    }
    return 0;
}

int csfs_data_volume_available(void) {
    if (access(CSFS_DATA_VOLUME, F_OK) != 0) {
        return 0;
    }
    return csfs_volume_supports_searchfs(CSFS_DATA_VOLUME);
}

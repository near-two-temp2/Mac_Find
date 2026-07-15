/*
 * searchfs_addon.mm — N-API wrapper around the macOS searchfs() syscall.
 *
 * Road_C (TypeScript) hybrid engine uses this as the FALLBACK path: when the
 * self-built binary index is missing or corrupt, the app searches the live
 * filesystem catalog directly, exactly like Road_A does.
 *
 * The searchfs() call sequence, packed structs and EBUSY / dual-volume handling
 * are adapted from the reference implementation:
 *   ../../Open_Ref/searchfs/main.m  (Sveinbjorn Thordarson, BSD-3-Clause)
 */

#import <Foundation/Foundation.h>

#include <napi.h>

#include <string>
#include <vector>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>
#include <string.h>
#include <sys/attr.h>
#include <sys/param.h>
#include <sys/vnode.h>
#include <sys/fsgetpath.h>
#include <sys/mount.h>

struct packed_name_attr {
    u_int32_t               size;
    struct attrreference    ref;
    char                    name[PATH_MAX];
};

struct packed_attr_ref {
    u_int32_t               size;
    struct attrreference    ref;
};

struct packed_result {
    u_int32_t           size;
    struct fsid         fs_id;
    struct fsobj_id     obj_id;
};

#define MAX_MATCHES         64
#define MAX_EBUSY_RETRIES   5
#define DATA_VOLUME         "/System/Volumes/Data"

// Runs searchfs() on one volume, appending matching paths to `out`.
// Mirrors do_searchfs_search() from the reference main.m but collects results
// into a vector instead of printing them, and honours a global result limit.
static void do_searchfs_search(const char *volpath,
                               const char *match_string,
                               bool dirsOnly,
                               bool filesOnly,
                               bool exactMatch,
                               size_t limit,
                               std::vector<std::string> &out) {
    int                     err = 0;
    int                     ebusy_count = 0;
    unsigned long           matches;
    unsigned int            search_options;
    struct fssearchblock    search_blk;
    struct attrlist         return_list;
    struct searchstate      search_state;
    struct packed_name_attr info1;
    struct packed_attr_ref  info2;
    struct packed_result    result_buffer[MAX_MATCHES];

catalog_changed:
    search_blk.searchattrs.bitmapcount = ATTR_BIT_MAP_COUNT;
    search_blk.searchattrs.reserved = 0;
    search_blk.searchattrs.commonattr = ATTR_CMN_NAME;
    search_blk.searchattrs.volattr = 0;
    search_blk.searchattrs.dirattr = 0;
    search_blk.searchattrs.fileattr = 0;
    search_blk.searchattrs.forkattr = 0;

    search_blk.returnattrs = &return_list;
    return_list.bitmapcount = ATTR_BIT_MAP_COUNT;
    return_list.reserved = 0;
    return_list.commonattr = ATTR_CMN_FSID | ATTR_CMN_OBJID;
    return_list.volattr = 0;
    return_list.dirattr = 0;
    return_list.fileattr = 0;
    return_list.forkattr = 0;

    search_blk.returnbuffer = result_buffer;
    search_blk.returnbuffersize = sizeof(result_buffer);

    strlcpy(info1.name, match_string, sizeof(info1.name));
    info1.ref.attr_dataoffset = sizeof(struct attrreference);
    info1.ref.attr_length = (u_int32_t)strlen(info1.name) + 1;
    info1.size = sizeof(struct attrreference) + info1.ref.attr_length;
    search_blk.searchparams1 = &info1;
    search_blk.sizeofsearchparams1 = info1.size + sizeof(u_int32_t);

    info2.size = sizeof(struct attrreference);
    info2.ref.attr_dataoffset = sizeof(struct attrreference);
    info2.ref.attr_length = 0;
    search_blk.searchparams2 = &info2;
    search_blk.sizeofsearchparams2 = sizeof(info2);

    search_blk.maxmatches = MAX_MATCHES;
    search_blk.timelimit.tv_sec = 1;
    search_blk.timelimit.tv_usec = 0;

    search_options = SRCHFS_START;
    if (!dirsOnly)  search_options |= SRCHFS_MATCHFILES;
    if (!filesOnly) search_options |= SRCHFS_MATCHDIRS;
    if (!exactMatch) search_options |= SRCHFS_MATCHPARTIALNAMES;

    do {
        err = searchfs(volpath, &search_blk, &matches, 0, search_options, &search_state);
        if (err == -1) {
            err = errno;
        }

        if ((err == 0 || err == EAGAIN) && matches > 0) {
            char *ptr = (char *)&result_buffer[0];
            char *end_ptr = (ptr + sizeof(result_buffer));

            for (unsigned int i = 0; i < matches; ++i) {
                struct packed_result *result_p = (struct packed_result *)ptr;

                char path_buf[PATH_MAX];
                ssize_t size = fsgetpath((char *)&path_buf,
                                         sizeof(path_buf),
                                         &result_p->fs_id,
                                         (uint64_t)result_p->obj_id.fid_objno |
                                         ((uint64_t)result_p->obj_id.fid_generation << 32));
                if (size > -1) {
                    out.emplace_back(path_buf);
                    if (limit && out.size() >= limit) {
                        return;
                    }
                }

                ptr = (ptr + result_p->size);
                if (ptr > end_ptr) {
                    break;
                }
            }
        }

        if ((err == EBUSY) && (ebusy_count++ < MAX_EBUSY_RETRIES)) {
            goto catalog_changed;
        }

        search_options &= ~SRCHFS_START;
    } while (err == EAGAIN);
}

// JS signature: search(pattern: string, opts?: { dirsOnly?, filesOnly?, exact?, limit? }) -> string[]
static Napi::Value Search(const Napi::CallbackInfo &info) {
    Napi::Env env = info.Env();

    if (info.Length() < 1 || !info[0].IsString()) {
        Napi::TypeError::New(env, "search(pattern) requires a string pattern").ThrowAsJavaScriptException();
        return env.Null();
    }

    std::string pattern = info[0].As<Napi::String>().Utf8Value();

    bool dirsOnly = false, filesOnly = false, exactMatch = false;
    size_t limit = 1000;

    if (info.Length() >= 2 && info[1].IsObject()) {
        Napi::Object o = info[1].As<Napi::Object>();
        if (o.Has("dirsOnly"))  dirsOnly  = o.Get("dirsOnly").ToBoolean().Value();
        if (o.Has("filesOnly")) filesOnly = o.Get("filesOnly").ToBoolean().Value();
        if (o.Has("exact"))     exactMatch = o.Get("exact").ToBoolean().Value();
        if (o.Has("limit"))     limit = (size_t)o.Get("limit").ToNumber().Int64Value();
    }

    std::vector<std::string> results;

    @autoreleasepool {
        // Root volume, then the Catalina+ Data volume (deduped by the caller if needed).
        do_searchfs_search("/", pattern.c_str(), dirsOnly, filesOnly, exactMatch, limit, results);

        if ((!limit || results.size() < limit) &&
            [[NSFileManager defaultManager] fileExistsAtPath:@DATA_VOLUME]) {
            size_t remaining = limit ? (limit - results.size()) : 0;
            do_searchfs_search(DATA_VOLUME, pattern.c_str(), dirsOnly, filesOnly, exactMatch, remaining, results);
        }
    }

    Napi::Array arr = Napi::Array::New(env, results.size());
    for (size_t i = 0; i < results.size(); ++i) {
        arr.Set((uint32_t)i, Napi::String::New(env, results[i]));
    }
    return arr;
}

static Napi::Object Init(Napi::Env env, Napi::Object exports) {
    exports.Set("search", Napi::Function::New(env, Search));
    return exports;
}

NODE_API_MODULE(searchfs_addon, Init)

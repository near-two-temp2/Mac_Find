/*
 * searchfs.cc — Node native addon (N-API) wrapping macOS searchfs(2).
 *
 * Exposes a single synchronous function `search(term, options)` to JS that
 * performs a filename catalog search on the root and Data volumes using the
 * macOS kernel searchfs() syscall (no index, real-time).
 *
 * Ported from Open_Ref/searchfs/main.m (BSD-3, Sveinbjorn Thordarson).
 * We keep the same fssearchblock layout / SRCHFS_* flag handling and the
 * fsgetpath() objid->path resolution, but hand results back to JS instead of
 * printing to stdout.
 *
 * Build target: macOS only. On non-macOS the addon is compiled as a stub so
 * CI on other platforms would still link (we only ever build on macos-latest).
 */

#include <napi.h>

#ifdef __APPLE__

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cerrno>
#include <unistd.h>
#include <sys/attr.h>
#include <sys/param.h>
#include <sys/vnode.h>
#include <sys/fsgetpath.h>
#include <sys/mount.h>
#include <string>
#include <vector>
#include <algorithm>
#include <cctype>

// --- packed structures (mirror main.m) ---------------------------------------

struct packed_name_attr {
    u_int32_t            size;      // Of the remaining fields
    struct attrreference ref;       // Offset/length of name itself
    char                 name[PATH_MAX];
};

struct packed_attr_ref {
    u_int32_t            size;      // Of the remaining fields
    struct attrreference ref;       // Offset/length of attr itself
};

struct packed_result {
    u_int32_t       size;           // Including size field itself
    struct fsid     fs_id;
    struct fsobj_id obj_id;
};

#define MAX_MATCHES        256
#define MAX_EBUSY_RETRIES  5
#define DEFAULT_VOLUME     "/"
#define DATA_VOLUME        "/System/Volumes/Data"

// Options passed in from JS, translated to SRCHFS_* flags.
struct SearchOpts {
    bool     dirsOnly       = false;
    bool     filesOnly      = false;
    bool     exactMatch     = false;
    bool     caseSensitive  = false;
    bool     skipPackages   = false;
    bool     skipInvisibles = false;
    uint64_t limit          = 0;    // 0 == unlimited
};

// Does the volume at `path` support catalog (searchfs) search?
static bool vol_supports_searchfs(const char *path) {
    struct vol_attr_buf {
        u_int32_t               size;
        vol_capabilities_attr_t vol_capabilities;
    } __attribute__((aligned(4), packed));

    struct attrlist attrList;
    memset(&attrList, 0, sizeof(attrList));
    attrList.bitmapcount = ATTR_BIT_MAP_COUNT;
    attrList.volattr = (ATTR_VOL_INFO | ATTR_VOL_CAPABILITIES);

    struct vol_attr_buf attrBuf;
    memset(&attrBuf, 0, sizeof(attrBuf));

    if (getattrlist(path, &attrList, &attrBuf, sizeof(attrBuf), 0) != 0) {
        return false;
    }
    if (attrBuf.size != sizeof(attrBuf)) {
        return false;
    }
    if ((attrBuf.vol_capabilities.valid[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS &&
        (attrBuf.vol_capabilities.capabilities[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS) {
        return true;
    }
    return false;
}

// Run searchfs() over one volume, appending resolved paths to `out`.
// Returns number of matches appended. Respects opts.limit as a *remaining*
// budget (caller decrements between volumes).
static uint64_t do_searchfs_search(const char *volpath,
                                   const char *match_string,
                                   const SearchOpts &opts,
                                   uint64_t remaining,
                                   std::vector<std::string> &out) {
    int                   err = 0;
    unsigned long         matches;
    unsigned int          search_options;
    struct fssearchblock  search_blk;
    struct attrlist       return_list;
    struct searchstate    search_state;
    struct packed_name_attr info1;
    struct packed_attr_ref  info2;
    static packed_result  result_buffer[MAX_MATCHES];
    uint64_t              match_cnt = 0;
    int                   ebusy_count = 0;

catalog_changed:
    search_blk.searchattrs.bitmapcount = ATTR_BIT_MAP_COUNT;
    search_blk.searchattrs.reserved    = 0;
    search_blk.searchattrs.commonattr  = ATTR_CMN_NAME;
    search_blk.searchattrs.volattr     = 0;
    search_blk.searchattrs.dirattr     = 0;
    search_blk.searchattrs.fileattr    = 0;
    search_blk.searchattrs.forkattr    = 0;

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

    // Name goes only in searchparams1.
    strncpy(info1.name, match_string, PATH_MAX - 1);
    info1.name[PATH_MAX - 1] = '\0';
    info1.ref.attr_dataoffset = sizeof(struct attrreference);
    info1.ref.attr_length     = (u_int32_t)strlen(info1.name) + 1;
    info1.size                = sizeof(struct attrreference) + info1.ref.attr_length;
    search_blk.searchparams1     = &info1;
    search_blk.sizeofsearchparams1 = info1.size + sizeof(u_int32_t);

    info2.size = sizeof(struct attrreference);
    info2.ref.attr_dataoffset = sizeof(struct attrreference);
    info2.ref.attr_length     = 0;
    search_blk.searchparams2     = &info2;
    search_blk.sizeofsearchparams2 = sizeof(info2);

    search_blk.maxmatches       = MAX_MATCHES;
    search_blk.timelimit.tv_sec  = 1;
    search_blk.timelimit.tv_usec = 0;

    search_options = SRCHFS_START;
    if (!opts.dirsOnly)        search_options |= SRCHFS_MATCHFILES;
    if (!opts.filesOnly)       search_options |= SRCHFS_MATCHDIRS;
    if (!opts.exactMatch)      search_options |= SRCHFS_MATCHPARTIALNAMES;
    if (opts.skipPackages)     search_options |= SRCHFS_SKIPPACKAGES;
    if (opts.skipInvisibles)   search_options |= SRCHFS_SKIPINVISIBLE;

    do {
        err = searchfs(volpath, &search_blk, &matches, 0, search_options, &search_state);
        if (err == -1) err = errno;

        if ((err == 0 || err == EAGAIN) && matches > 0) {
            char *ptr     = (char *)&result_buffer[0];
            char *end_ptr = ptr + sizeof(result_buffer);

            for (unsigned long i = 0; i < matches; ++i) {
                packed_result *result_p = (packed_result *)ptr;

                char path_buf[PATH_MAX];
                ssize_t size = fsgetpath(path_buf,
                                         sizeof(path_buf),
                                         &result_p->fs_id,
                                         (uint64_t)result_p->obj_id.fid_objno |
                                         ((uint64_t)result_p->obj_id.fid_generation << 32));
                if (size > -1) {
                    bool keep = true;
                    // When the caller asks for case-sensitive matching, the
                    // kernel's insensitive match is too loose — filter here.
                    if (opts.caseSensitive) {
                        std::string p(path_buf);
                        std::string basename = p;
                        size_t slash = p.find_last_of('/');
                        if (slash != std::string::npos) basename = p.substr(slash + 1);
                        if (basename.find(match_string) == std::string::npos) {
                            keep = false;
                        }
                    }
                    if (keep) {
                        out.emplace_back(path_buf);
                        match_cnt++;
                        if (remaining && match_cnt >= remaining) {
                            return match_cnt;
                        }
                    }
                }

                ptr += result_p->size;
                if (ptr > end_ptr) break;
            }
        }

        if (err == EBUSY && ebusy_count++ < MAX_EBUSY_RETRIES) {
            goto catalog_changed;
        }

        search_options &= ~SRCHFS_START;
    } while (err == EAGAIN);

    return match_cnt;
}

// JS-facing: search(term: string, options: object) -> string[]
Napi::Value Search(const Napi::CallbackInfo &info) {
    Napi::Env env = info.Env();

    if (info.Length() < 1 || !info[0].IsString()) {
        Napi::TypeError::New(env, "search(term, options): term must be a string")
            .ThrowAsJavaScriptException();
        return env.Null();
    }

    std::string term = info[0].As<Napi::String>().Utf8Value();

    SearchOpts opts;
    if (info.Length() >= 2 && info[1].IsObject()) {
        Napi::Object o = info[1].As<Napi::Object>();
        auto getBool = [&](const char *k, bool def) -> bool {
            return o.Has(k) && o.Get(k).IsBoolean() ? o.Get(k).As<Napi::Boolean>().Value() : def;
        };
        opts.dirsOnly       = getBool("dirsOnly", false);
        opts.filesOnly      = getBool("filesOnly", false);
        opts.exactMatch     = getBool("exactMatch", false);
        opts.caseSensitive  = getBool("caseSensitive", false);
        opts.skipPackages   = getBool("skipPackages", false);
        opts.skipInvisibles = getBool("skipInvisibles", false);
        if (o.Has("limit") && o.Get("limit").IsNumber()) {
            double l = o.Get("limit").As<Napi::Number>().DoubleValue();
            if (l > 0) opts.limit = (uint64_t)l;
        }
    }

    std::vector<std::string> results;

    if (term.empty() || term.size() >= PATH_MAX) {
        return Napi::Array::New(env, 0);
    }

    // Root volume first.
    uint64_t remaining = opts.limit;
    uint64_t found = 0;
    if (vol_supports_searchfs(DEFAULT_VOLUME)) {
        found += do_searchfs_search(DEFAULT_VOLUME, term.c_str(), opts, remaining, results);
    }

    // Then the Data volume (Catalina+ split), unless we've hit the limit.
    if (!opts.limit || found < opts.limit) {
        if (access(DATA_VOLUME, F_OK) == 0 && vol_supports_searchfs(DATA_VOLUME)) {
            uint64_t rem2 = opts.limit ? (opts.limit - found) : 0;
            found += do_searchfs_search(DATA_VOLUME, term.c_str(), opts, rem2, results);
        }
    }

    Napi::Array arr = Napi::Array::New(env, results.size());
    for (size_t i = 0; i < results.size(); ++i) {
        arr.Set((uint32_t)i, Napi::String::New(env, results[i]));
    }
    return arr;
}

#else // !__APPLE__ — stub so the addon still compiles on non-macOS.

Napi::Value Search(const Napi::CallbackInfo &info) {
    Napi::Env env = info.Env();
    Napi::Error::New(env, "searchfs is only available on macOS")
        .ThrowAsJavaScriptException();
    return env.Null();
}

#endif

Napi::Object Init(Napi::Env env, Napi::Object exports) {
    exports.Set("search", Napi::Function::New(env, Search));
    return exports;
}

NODE_API_MODULE(searchfs, Init)

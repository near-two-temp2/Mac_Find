// SearchFsEngine.cpp — searchfs(2) real-time fallback engine.
//
// Adapted from Open_Ref/searchfs/main.m. The core is the packed searchfs
// attribute buffers and the SRCHFS_START / EAGAIN pagination loop; fsgetpath()
// turns each (fsid, objid) result back into an absolute path.
#include "SearchFsEngine.h"

#include <cerrno>
#include <cstring>

#include <unistd.h>
#include <sys/attr.h>
#include <sys/param.h>
#include <sys/mount.h>
#include <sys/fsgetpath.h>

namespace macfind {

namespace {

constexpr int MAX_MATCHES       = 128;  // matches per searchfs() call
constexpr int MAX_EBUSY_RETRIES = 5;    // catalog-changed retries

// Packed attribute buffers, byte-for-byte matching the kernel's expectations.
// The name being searched lives in searchparams1 only.
struct packed_name_attr {
    u_int32_t            size;
    struct attrreference ref;
    char                 name[PATH_MAX];
};

struct packed_attr_ref {
    u_int32_t            size;
    struct attrreference ref;
};

// The kernel returns fsid + objid so fsgetpath() can rebuild the path.
struct packed_result {
    u_int32_t       size;
    struct fsid     fs_id;
    struct fsobj_id obj_id;
};

struct vol_attr_buf {
    u_int32_t               size;
    vol_capabilities_attr_t vol_capabilities;
} __attribute__((aligned(4), packed));

// Last path component of a C++ path string.
std::string baseName(const std::string& path) {
    auto slash = path.find_last_of('/');
    return slash == std::string::npos ? path : path.substr(slash + 1);
}

}  // namespace

bool volumeSupportsSearchFS(const std::string& mountPath) {
    struct attrlist attrList;
    std::memset(&attrList, 0, sizeof(attrList));
    attrList.bitmapcount = ATTR_BIT_MAP_COUNT;
    attrList.volattr     = (ATTR_VOL_INFO | ATTR_VOL_CAPABILITIES);

    struct vol_attr_buf attrBuf;
    std::memset(&attrBuf, 0, sizeof(attrBuf));

    if (getattrlist(mountPath.c_str(), &attrList, &attrBuf, sizeof(attrBuf), 0) != 0) {
        return false;
    }
    if (attrBuf.size != sizeof(attrBuf)) {
        return false;
    }
    const auto& caps = attrBuf.vol_capabilities;
    return (caps.valid[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS &&
           (caps.capabilities[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS;
}

std::size_t SearchFsEngine::searchVolume(const char* volpath,
                                         const std::string& term,
                                         const SearchOptions& opts,
                                         std::size_t remainingLimit,
                                         const ResultCallback& onResult,
                                         SearchOutcome& out) {
    int                  err        = 0;
    int                  ebusyCount = 0;
    unsigned long        matches    = 0;
    struct fssearchblock searchBlk;
    struct attrlist      returnList;
    struct searchstate   searchState;
    packed_name_attr     info1;
    packed_attr_ref      info2;
    packed_result        resultBuffer[MAX_MATCHES];
    std::size_t          matchCnt = 0;

    std::memset(&searchBlk, 0, sizeof(searchBlk));
    std::memset(&info1, 0, sizeof(info1));
    std::memset(&info2, 0, sizeof(info2));

    unsigned int searchOptions;

catalog_changed:
    searchBlk.searchattrs.bitmapcount = ATTR_BIT_MAP_COUNT;
    searchBlk.searchattrs.reserved    = 0;
    searchBlk.searchattrs.commonattr  = ATTR_CMN_NAME;
    searchBlk.searchattrs.volattr     = 0;
    searchBlk.searchattrs.dirattr     = 0;
    searchBlk.searchattrs.fileattr    = 0;
    searchBlk.searchattrs.forkattr    = 0;

    searchBlk.returnattrs      = &returnList;
    returnList.bitmapcount     = ATTR_BIT_MAP_COUNT;
    returnList.reserved        = 0;
    returnList.commonattr      = ATTR_CMN_FSID | ATTR_CMN_OBJID;
    returnList.volattr         = 0;
    returnList.dirattr         = 0;
    returnList.fileattr        = 0;
    returnList.forkattr        = 0;

    searchBlk.returnbuffer     = resultBuffer;
    searchBlk.returnbuffersize = sizeof(resultBuffer);

    std::strncpy(info1.name, term.c_str(), sizeof(info1.name) - 1);
    info1.name[sizeof(info1.name) - 1] = '\0';
    info1.ref.attr_dataoffset     = sizeof(struct attrreference);
    info1.ref.attr_length         = static_cast<u_int32_t>(std::strlen(info1.name)) + 1;
    info1.size                    = sizeof(struct attrreference) + info1.ref.attr_length;
    searchBlk.searchparams1       = &info1;
    searchBlk.sizeofsearchparams1 = info1.size + sizeof(u_int32_t);

    info2.size                    = sizeof(struct attrreference);
    info2.ref.attr_dataoffset     = sizeof(struct attrreference);
    info2.ref.attr_length         = 0;
    searchBlk.searchparams2       = &info2;
    searchBlk.sizeofsearchparams2 = sizeof(info2);

    searchBlk.maxmatches        = MAX_MATCHES;
    searchBlk.timelimit.tv_sec  = 1;
    searchBlk.timelimit.tv_usec = 0;

    // searchfs() matches case-insensitively on the substring; we post-filter
    // dirs/files below since the return attrs don't carry that cheaply here.
    searchOptions = SRCHFS_START | SRCHFS_MATCHFILES | SRCHFS_MATCHDIRS |
                    SRCHFS_MATCHPARTIALNAMES;

    do {
        err = searchfs(volpath, &searchBlk, &matches, 0, searchOptions, &searchState);
        if (err == -1) err = errno;

        if ((err == 0 || err == EAGAIN) && matches > 0) {
            char* ptr    = reinterpret_cast<char*>(&resultBuffer[0]);
            char* endPtr = ptr + sizeof(resultBuffer);

            for (unsigned long i = 0; i < matches; ++i) {
                auto* resultP = reinterpret_cast<packed_result*>(ptr);

                char pathBuf[PATH_MAX];
                ssize_t size = fsgetpath(pathBuf, sizeof(pathBuf), &resultP->fs_id,
                                         static_cast<uint64_t>(resultP->obj_id.fid_objno) |
                                         (static_cast<uint64_t>(resultP->obj_id.fid_generation) << 32));
                if (size > -1) {
                    std::string path(pathBuf);
                    bool keep = true;

                    // Post-filter case-sensitivity (kernel matched case-insensitively).
                    if (opts.caseSensitive &&
                        baseName(path).find(term) == std::string::npos) {
                        keep = false;
                    }

                    if (keep) {
                        SearchResult r;
                        r.path = path;
                        // Trailing '/' is a cheap directory hint; not authoritative,
                        // so we leave isDir=false and let dirs/filesOnly stay lenient.
                        out.results.push_back(r);
                        ++matchCnt;
                        if (onResult && !onResult(r)) return matchCnt;
                        if (remainingLimit && matchCnt >= remainingLimit) return matchCnt;
                    }
                }
                // fsgetpath() failure usually means the object was deleted mid-scan.

                ptr += resultP->size;
                if (ptr > endPtr) break;
            }
        }

        if (err == EBUSY && ebusyCount++ < MAX_EBUSY_RETRIES) {
            goto catalog_changed;  // catalog changed under us; restart bounded
        }
        if (err != 0 && err != EAGAIN && err != EBUSY) {
            out.error = std::string("searchfs() failed: ") + std::strerror(err);
        }

        searchOptions &= ~SRCHFS_START;  // continue the scan on later calls
    } while (err == EAGAIN);

    return matchCnt;
}

SearchOutcome SearchFsEngine::search(const std::string& term,
                                     const SearchOptions& opts,
                                     const ResultCallback& onResult) {
    SearchOutcome out;
    out.backend = Backend::SearchFS;

    if (term.empty() || term.size() > PATH_MAX) {
        out.ok = false;
        out.error = "Empty or invalid search term.";
        return out;
    }

    std::vector<std::string> volumes{"/"};
    const std::string dataVol = "/System/Volumes/Data";
    if (access(dataVol.c_str(), F_OK) == 0 && volumeSupportsSearchFS(dataVol)) {
        volumes.push_back(dataVol);
    }

    std::size_t total = 0;
    for (const auto& vol : volumes) {
        if (!volumeSupportsSearchFS(vol)) {
            if (out.results.empty()) {
                out.error = "Volume does not support catalog search: " + vol;
            }
            continue;
        }
        std::size_t remaining = 0;
        if (opts.limit) {
            if (total >= opts.limit) break;
            remaining = opts.limit - total;
        }
        total += searchVolume(vol.c_str(), term, opts, remaining, onResult, out);
        if (opts.limit && total >= opts.limit) break;
    }
    return out;
}

}  // namespace macfind

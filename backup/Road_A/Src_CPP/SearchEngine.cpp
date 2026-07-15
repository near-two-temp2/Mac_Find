// SearchEngine.cpp — searchfs(2) real-time filename search engine.
//
// Adapted from Open_Ref/searchfs/main.m. The heavy lifting is the packed
// searchfs attribute buffers and the SRCHFS_START / EAGAIN pagination loop.
#include "SearchEngine.h"

#include <cerrno>
#include <cstdio>
#include <cstring>
#include <cctype>

#include <unistd.h>
#include <sys/attr.h>
#include <sys/param.h>
#include <sys/mount.h>
#include <sys/fsgetpath.h>

namespace macfind {

namespace {

constexpr int MAX_MATCHES       = 128;  // matches per searchfs() call
constexpr int MAX_EBUSY_RETRIES = 5;    // catalog-changed retries

// Packed attribute buffers, byte-for-byte matching the kernel's expectations
// (see main.m). Names live in searchparams1 only.
struct packed_name_attr {
    u_int32_t            size;            // size of the remaining fields
    struct attrreference ref;            // offset/length of name itself
    char                 name[PATH_MAX];
};

struct packed_attr_ref {
    u_int32_t            size;
    struct attrreference ref;
};

struct packed_result {
    u_int32_t       size;                // including size field itself
    struct fsid     fs_id;
    struct fsobj_id obj_id;
};

// Volume capability probe buffer.
struct vol_attr_buf {
    u_int32_t               size;
    vol_capabilities_attr_t vol_capabilities;
} __attribute__((aligned(4), packed));

// Case-insensitive substring test used for the post-filter path (case-sensitive
// mode) — the kernel already handles the case-insensitive substring match.
bool containsCaseSensitive(const std::string& hay, const std::string& needle) {
    if (needle.empty()) return true;
    return hay.find(needle) != std::string::npos;
}

// Extract the basename (last path component) of a C path.
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
    if ((caps.valid[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS &&
        (caps.capabilities[VOL_CAPABILITIES_INTERFACES] & VOL_CAP_INT_SEARCHFS) == VOL_CAP_INT_SEARCHFS) {
        return true;
    }
    return false;
}

std::vector<std::string> listSearchableVolumes() {
    std::vector<std::string> out;
    int fsCount = getfsstat(nullptr, 0, MNT_NOWAIT);
    if (fsCount <= 0) return out;

    std::vector<struct statfs> buf(static_cast<std::size_t>(fsCount));
    int actual = getfsstat(buf.data(), fsCount * static_cast<int>(sizeof(struct statfs)), MNT_NOWAIT);
    if (actual <= 0) return out;

    for (int i = 0; i < actual; ++i) {
        if (volumeSupportsSearchFS(buf[i].f_mntonname)) {
            out.emplace_back(buf[i].f_mntonname);
        }
    }
    return out;
}

std::size_t SearchEngine::searchVolume(const char* volpath,
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
    // Search key: we only search by ATTR_CMN_NAME.
    searchBlk.searchattrs.bitmapcount = ATTR_BIT_MAP_COUNT;
    searchBlk.searchattrs.reserved    = 0;
    searchBlk.searchattrs.commonattr  = ATTR_CMN_NAME;
    searchBlk.searchattrs.volattr     = 0;
    searchBlk.searchattrs.dirattr     = 0;
    searchBlk.searchattrs.fileattr    = 0;
    searchBlk.searchattrs.forkattr    = 0;

    // We want fsid + objid back so fsgetpath() can restore the path.
    searchBlk.returnattrs        = &returnList;
    returnList.bitmapcount       = ATTR_BIT_MAP_COUNT;
    returnList.reserved          = 0;
    returnList.commonattr        = ATTR_CMN_FSID | ATTR_CMN_OBJID;
    returnList.volattr           = 0;
    returnList.dirattr           = 0;
    returnList.fileattr          = 0;
    returnList.forkattr          = 0;

    searchBlk.returnbuffer       = resultBuffer;
    searchBlk.returnbuffersize   = sizeof(resultBuffer);

    // Pack searchparams1: the name to match (bounded to PATH_MAX-1).
    std::strncpy(info1.name, term.c_str(), sizeof(info1.name) - 1);
    info1.name[sizeof(info1.name) - 1] = '\0';
    info1.ref.attr_dataoffset    = sizeof(struct attrreference);
    info1.ref.attr_length        = static_cast<u_int32_t>(std::strlen(info1.name)) + 1;
    info1.size                   = sizeof(struct attrreference) + info1.ref.attr_length;
    searchBlk.searchparams1      = &info1;
    searchBlk.sizeofsearchparams1 = info1.size + sizeof(u_int32_t);

    // searchparams2 is unused but must be a valid empty attr ref.
    info2.size                   = sizeof(struct attrreference);
    info2.ref.attr_dataoffset    = sizeof(struct attrreference);
    info2.ref.attr_length        = 0;
    searchBlk.searchparams2      = &info2;
    searchBlk.sizeofsearchparams2 = sizeof(info2);

    searchBlk.maxmatches         = MAX_MATCHES;
    searchBlk.timelimit.tv_sec   = 1;
    searchBlk.timelimit.tv_usec  = 0;

    searchOptions = SRCHFS_START;
    if (!opts.dirsOnly)       searchOptions |= SRCHFS_MATCHFILES;
    if (!opts.filesOnly)      searchOptions |= SRCHFS_MATCHDIRS;
    if (!opts.exactMatch)     searchOptions |= SRCHFS_MATCHPARTIALNAMES;
    if (opts.skipPackages)    searchOptions |= SRCHFS_SKIPPACKAGES;
    if (opts.skipInvisibles)  searchOptions |= SRCHFS_SKIPINVISIBLE;

    do {
        err = searchfs(volpath, &searchBlk, &matches, 0, searchOptions, &searchState);
        if (err == -1) {
            err = errno;
        }

        if ((err == 0 || err == EAGAIN) && matches > 0) {
            char* ptr     = reinterpret_cast<char*>(&resultBuffer[0]);
            char* endPtr  = ptr + sizeof(resultBuffer);

            for (unsigned long i = 0; i < matches; ++i) {
                auto* resultP = reinterpret_cast<packed_result*>(ptr);

                char pathBuf[PATH_MAX];
                ssize_t size = fsgetpath(pathBuf, sizeof(pathBuf), &resultP->fs_id,
                                         static_cast<uint64_t>(resultP->obj_id.fid_objno) |
                                         (static_cast<uint64_t>(resultP->obj_id.fid_generation) << 32));
                if (size > -1) {
                    std::string path(pathBuf);
                    bool keep = true;

                    // Post-filter for case-sensitive mode: the kernel matches
                    // case-insensitively, so drop non-matching results here.
                    if (opts.caseSensitive &&
                        !containsCaseSensitive(baseName(path), term)) {
                        keep = false;
                    }

                    if (keep) {
                        SearchResult r{path};
                        out.results.push_back(r);
                        ++matchCnt;
                        if (onResult && !onResult(r)) {
                            return matchCnt;  // caller asked to stop
                        }
                        if (remainingLimit && matchCnt >= remainingLimit) {
                            return matchCnt;
                        }
                    }
                }
                // If fsgetpath failed the object was likely deleted mid-search;
                // skip it silently.

                ptr += resultP->size;
                if (ptr > endPtr) break;
            }
        }

        // EBUSY == catalog changed underneath us; restart a bounded number of times.
        if (err == EBUSY && ebusyCount++ < MAX_EBUSY_RETRIES) {
            goto catalog_changed;
        }

        if (err != 0 && err != EAGAIN && err != EBUSY) {
            // Non-recoverable for this volume. Record but don't abort other volumes.
            out.error = std::string("searchfs() failed: ") + std::strerror(err);
        }

        searchOptions &= ~SRCHFS_START;  // subsequent calls continue the scan
    } while (err == EAGAIN);

    return matchCnt;
}

SearchOutcome SearchEngine::search(const std::string& term,
                                   const SearchOptions& opts,
                                   const std::string& volumePath,
                                   const ResultCallback& onResult) {
    SearchOutcome out;

    if (term.empty() || term.size() > PATH_MAX) {
        out.ok = false;
        out.error = "Empty or invalid search term.";
        return out;
    }

    // Determine which volumes to search.
    std::vector<std::string> volumes;
    if (!volumePath.empty()) {
        volumes.push_back(volumePath);
    } else {
        volumes.push_back("/");
        // Catalina+ splits the system into a read-only "/" and a writable data
        // volume at /System/Volumes/Data. Search both for full coverage.
        const std::string dataVol = "/System/Volumes/Data";
        if (access(dataVol.c_str(), F_OK) == 0 && volumeSupportsSearchFS(dataVol)) {
            volumes.push_back(dataVol);
        }
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

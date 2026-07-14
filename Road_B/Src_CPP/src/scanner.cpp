#include "scanner.hpp"

#include <fts.h>
#include <cstdlib>
#include <cstring>

namespace mff {

std::vector<std::string> defaultRoots() {
    const char* home = std::getenv("HOME");
    if (home && home[0]) return {std::string(home)};
    return {"/tmp"};
}

// Directory names we skip entirely: massive, churny, and never interesting to
// search. Matched on the leaf name.
static bool skipDir(const char* name) {
    static const char* kSkip[] = {
        ".git", "node_modules", ".Trash", "Caches", ".cache",
        "DerivedData", ".build", ".gradle",
    };
    for (const char* s : kSkip)
        if (std::strcmp(name, s) == 0) return true;
    return false;
}

size_t scanRoots(const std::vector<std::string>& roots, const ScanSink& sink) {
    // fts_open wants a NULL-terminated argv-style array of C strings.
    std::vector<char*> argv;
    argv.reserve(roots.size() + 1);
    for (const auto& r : roots) argv.push_back(const_cast<char*>(r.c_str()));
    argv.push_back(nullptr);

    // FTS_PHYSICAL: don't follow symlinks (avoids cycles/duplication).
    // FTS_NOSTAT: skip stat() on every node for speed.
    // FTS_NOCHDIR: keep absolute paths in fts_path.
    FTS* fts = fts_open(argv.data(),
                        FTS_PHYSICAL | FTS_NOSTAT | FTS_NOCHDIR, nullptr);
    if (!fts) return 0;

    size_t count = 0;
    FTSENT* ent;
    while ((ent = fts_read(fts)) != nullptr) {
        switch (ent->fts_info) {
            case FTS_D: // directory, pre-order
                if (skipDir(ent->fts_name)) {
                    fts_set(fts, ent, FTS_SKIP); // prune this subtree
                    break;
                }
                sink(std::string(ent->fts_path, ent->fts_pathlen), true);
                ++count;
                break;
            // Under FTS_NOSTAT non-directories arrive as FTS_NSOK (no stat
            // performed). fts still resolves directories to FTS_D/FTS_DP so it
            // can recurse, so anything reaching here is a file/symlink.
            case FTS_NSOK:
            case FTS_F:   // regular file
            case FTS_SL:  // symlink (recorded but not followed)
            case FTS_SLNONE:
            case FTS_DEFAULT:
                sink(std::string(ent->fts_path, ent->fts_pathlen), false);
                ++count;
                break;
            default:
                break; // FTS_DP (post-order), FTS_DNR, FTS_ERR, etc.
        }
    }
    fts_close(fts);
    return count;
}

} // namespace mff

// SearchTypes.h — types shared across the Road_C (C++) hybrid engine.
//
// Road_C is the "complete" route: a self-built binary index (bitmask prefilter
// + fzf scoring) is the primary engine, and searchfs() is the fallback used when
// the index is missing or corrupt. Both engines and the coordinator share these
// plain option/result structs so the Qt GUI and the CLI can drive them the same.
#pragma once

#include <string>
#include <vector>
#include <cstddef>
#include <cstdint>

namespace macfind {

// User-facing search options (mirror the Road_A/Road_B flag surface).
struct SearchOptions {
    bool        dirsOnly      = false;  // match directories only
    bool        filesOnly     = false;  // match files only
    bool        caseSensitive = false;  // case-sensitive matching
    std::size_t limit         = 0;      // stop after N results (0 = unlimited)
};

// One matched filesystem object, with the fzf score when it came from the index.
struct SearchResult {
    std::string path;      // absolute path
    bool        isDir = false;
    int         score = 0; // fzf score (higher = better); 0 for searchfs matches
};

// Which backend actually produced a result set — surfaced in the GUI status bar
// so the hybrid behaviour is observable.
enum class Backend {
    Index,     // self-built binary index (primary)
    SearchFS,  // searchfs() real-time fallback
    None,      // nothing ran (e.g. empty query)
};

inline const char* backendName(Backend b) {
    switch (b) {
        case Backend::Index:    return "index";
        case Backend::SearchFS: return "searchfs";
        case Backend::None:     return "none";
    }
    return "?";
}

// Full outcome of a query.
struct SearchOutcome {
    std::vector<SearchResult> results;
    Backend                   backend = Backend::None;
    bool                      ok      = true;   // false only on a fatal error
    std::string               error;            // human-readable when !ok
};

}  // namespace macfind

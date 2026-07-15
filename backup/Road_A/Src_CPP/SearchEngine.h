// SearchEngine.h — Road_A (C++): real-time filename search via macOS searchfs().
//
// Pure C++ / POSIX wrapper around the searchfs(2) system call. No Qt here so
// the same engine backs both the Qt6 GUI and the CLI smoke-test entry point.
//
// Ported/adapted from Open_Ref/searchfs/main.m (BSD-3, Sveinbjorn Thordarson).
#pragma once

#include <string>
#include <vector>
#include <cstdint>
#include <functional>

namespace macfind {

// Options mirroring the reference CLI flags (searchfs SRCHFS_* semantics).
struct SearchOptions {
    bool dirsOnly       = false;  // match directories only
    bool filesOnly      = false;  // match files only
    bool exactMatch     = false;  // exact filename match (no substring)
    bool caseSensitive  = false;  // case-sensitive matching (post-filter)
    bool skipPackages   = false;  // don't descend into .app/.bundle packages
    bool skipInvisibles = false;  // skip invisible files / dirs
    std::size_t limit   = 0;      // stop after N matches (0 = unlimited)
};

// One matched filesystem object.
struct SearchResult {
    std::string path;  // absolute path, restored via fsgetpath()
};

// Result of a full search across all searched volumes.
struct SearchOutcome {
    std::vector<SearchResult> results;
    bool ok = true;            // false if a fatal error occurred
    std::string error;         // human-readable message when ok == false
};

// List mounted volumes that support catalog search (for diagnostics / CLI -l).
std::vector<std::string> listSearchableVolumes();

// Whether a mount path's volume advertises VOL_CAP_INT_SEARCHFS.
bool volumeSupportsSearchFS(const std::string& mountPath);

// Callback invoked for every match as it is found (streaming). Return false to
// abort the search early. May be null.
using ResultCallback = std::function<bool(const SearchResult&)>;

class SearchEngine {
public:
    // Search `term` across the given volume mount path. If `volumePath` is
    // empty, searches "/" and, on Catalina+, also "/System/Volumes/Data".
    // `onResult` (if set) is called for each match as it streams in.
    SearchOutcome search(const std::string& term,
                         const SearchOptions& opts,
                         const std::string& volumePath = std::string(),
                         const ResultCallback& onResult = nullptr);

private:
    // Search a single volume; appends to `out`. Returns number of matches added.
    std::size_t searchVolume(const char* volpath,
                             const std::string& term,
                             const SearchOptions& opts,
                             std::size_t remainingLimit,
                             const ResultCallback& onResult,
                             SearchOutcome& out);
};

}  // namespace macfind

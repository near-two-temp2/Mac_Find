// SearchFsEngine.h — searchfs(2) real-time fallback engine for Road_C.
//
// This is the "备" (backup) half of the hybrid: when the binary index is
// missing or fails to load, the coordinator falls back to a live searchfs()
// scan so results stay 100% accurate at the cost of latency.
//
// Ported from Open_Ref/searchfs/main.m (BSD-3, Sveinbjorn Thordarson) and
// Road_A/Src_CPP/SearchEngine.
#pragma once

#include "SearchTypes.h"

#include <functional>
#include <string>

namespace macfind {

// Whether a mount path's volume advertises VOL_CAP_INT_SEARCHFS.
bool volumeSupportsSearchFS(const std::string& mountPath);

// Streaming callback: return false to abort the scan early. May be null.
using ResultCallback = std::function<bool(const SearchResult&)>;

class SearchFsEngine {
public:
    // Substring-match `term` across "/" and, on Catalina+, the data volume.
    // `onResult` (if set) fires for every match as it streams in.
    SearchOutcome search(const std::string& term,
                         const SearchOptions& opts,
                         const ResultCallback& onResult = nullptr);

private:
    std::size_t searchVolume(const char* volpath,
                             const std::string& term,
                             const SearchOptions& opts,
                             std::size_t remainingLimit,
                             const ResultCallback& onResult,
                             SearchOutcome& out);
};

}  // namespace macfind

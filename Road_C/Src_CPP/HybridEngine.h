// HybridEngine.h — Road_C coordinator: index-first, searchfs() fallback.
//
// This is what makes Road_C the "完整混合版": the self-built binary index is the
// primary path (fast, fuzzy, ranked); if the index can't be loaded (never built,
// deleted, or corrupt) the coordinator transparently falls back to a live
// searchfs() scan so the user always gets results.
//
// The SearchOutcome carries which Backend actually served the query so the GUI
// can show it in the status bar.
#pragma once

#include "SearchTypes.h"
#include "IndexEngine.h"
#include "SearchFsEngine.h"

#include <memory>
#include <string>

namespace macfind {

class HybridEngine {
public:
    HybridEngine();

    // Try to load the on-disk index from `indexPath` (defaults to
    // defaultIndexPath()). Safe to call repeatedly; returns whether an index is
    // now available. A failure here is not fatal — queries just use searchfs().
    bool loadIndex(const std::string& indexPath = std::string());

    // Build a fresh index over `roots` (default: $HOME) and persist it to
    // `indexPath`. On success the engine switches to using it.
    bool buildIndex(const std::vector<std::string>& roots = {},
                    const std::string& indexPath = std::string(),
                    const std::function<void(std::size_t)>& progress = nullptr);

    bool indexAvailable() const { return index_.loaded(); }
    std::size_t indexEntryCount() const { return index_.entryCount(); }

    // Primary search entry point. Uses the index when available; otherwise (or if
    // the index yields nothing usable and `allowFallback` is set) uses searchfs().
    // The chosen backend is recorded in the returned SearchOutcome.
    SearchOutcome search(const std::string& term,
                         const SearchOptions& opts,
                         bool allowFallback = true);

private:
    IndexEngine    index_;
    SearchFsEngine searchfs_;
    std::string    indexPath_;
};

}  // namespace macfind

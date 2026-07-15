// HybridEngine.cpp — index-first coordinator with searchfs() fallback.
#include "HybridEngine.h"

namespace macfind {

HybridEngine::HybridEngine() : indexPath_(defaultIndexPath()) {
    // Opportunistically load an existing index so the first query is fast.
    loadIndex(indexPath_);
}

bool HybridEngine::loadIndex(const std::string& indexPath) {
    if (!indexPath.empty()) indexPath_ = indexPath;
    // load() returns false on missing/corrupt files; we swallow that and let the
    // caller decide whether to build. Queries fall back to searchfs() regardless.
    return index_.load(indexPath_);
}

bool HybridEngine::buildIndex(const std::vector<std::string>& roots,
                              const std::string& indexPath,
                              const std::function<void(std::size_t)>& progress) {
    if (!indexPath.empty()) indexPath_ = indexPath;
    if (!index_.build(roots, progress)) return false;
    // Persist so subsequent launches skip the rescan. A save failure isn't fatal:
    // the in-memory index still serves this session.
    index_.save(indexPath_);
    return true;
}

SearchOutcome HybridEngine::search(const std::string& term,
                                   const SearchOptions& opts,
                                   bool allowFallback) {
    // Primary: the self-built index.
    if (index_.loaded()) {
        SearchOutcome out = index_.query(term, opts);
        if (out.ok) {
            // Index served the query. We do NOT auto-fall-back on "0 results":
            // an empty result from a fresh index is a real answer, and a live
            // searchfs() scan on every empty query would defeat the point.
            return out;
        }
        // ok == false means the index was unusable mid-query → fall through.
    }

    // Fallback: live searchfs() scan (index missing / corrupt / not built).
    if (allowFallback) {
        SearchOutcome out = searchfs_.search(term, opts);
        return out;
    }

    // Fallback disabled and no index: report the situation honestly.
    SearchOutcome out;
    out.backend = Backend::None;
    out.ok = false;
    out.error = "No index available and searchfs() fallback disabled.";
    return out;
}

}  // namespace macfind

// index_engine.hpp — build, persist, mmap-load and query the binary index.
//
// Road_B engine (open-source-analysis.md §3): a Cling-style parallel-array
// index with a UInt64 letter bitmask + extension id for O(n) Phase-1
// prefiltering, followed by fzf scoring in Phase-2. Phase-1 runs in parallel
// across CPU cores.

#pragma once

#include "index_format.hpp"

#include <cstdint>
#include <string>
#include <vector>

namespace mff {

struct SearchHit {
    std::string path;
    int         score;
    bool        isDir;
};

struct SearchOptions {
    int  maxResults    = 200;
    bool filesOnly     = false;
    bool dirsOnly      = false;
    // Optional extension filter, e.g. "pdf" (no dot). Empty = any.
    std::string extension;
};

// Owns the index either as freshly built vectors (after build/load-into-memory)
// or as an mmap'd view of a .idx file. Search works identically for both.
class IndexEngine {
public:
    IndexEngine() = default;
    ~IndexEngine();

    IndexEngine(const IndexEngine&) = delete;
    IndexEngine& operator=(const IndexEngine&) = delete;

    // Build an in-memory index from the given roots. Returns entry count.
    size_t buildFromRoots(const std::vector<std::string>& roots);

    // Serialize the current in-memory index to `path` (.idx). Requires that the
    // index was produced by buildFromRoots. Returns false on I/O error.
    bool save(const std::string& path) const;

    // mmap an existing .idx file for zero-copy search. Returns false if the file
    // is missing or has a bad magic/size. Replaces any current contents.
    bool loadMmap(const std::string& path);

    // Two-phase search. Query is matched case-insensitively.
    std::vector<SearchHit> search(const std::string& query,
                                  const SearchOptions& opts) const;

    size_t entryCount() const { return entryCount_; }
    bool   empty() const { return entryCount_ == 0; }

private:
    // Resolve a pointer to path bytes for entry i.
    const uint8_t* pathBytes(size_t i) const {
        return allBytes_ + byteOffsets_[i];
    }

    void reset();

    // --- Parallel arrays (either owned vectors or mmap'd pointers) ---
    size_t          entryCount_ = 0;
    const uint64_t* masks_       = nullptr;
    const uint64_t* bnMasks_     = nullptr;
    const uint64_t* bnBoundaries_= nullptr;
    const uint32_t* byteOffsets_ = nullptr;
    const uint16_t* byteLengths_ = nullptr;
    const uint16_t* bnStarts_    = nullptr;
    const uint16_t* extIds_      = nullptr;
    const uint8_t*  segCounts_   = nullptr;
    const uint8_t*  isDirs_      = nullptr;
    const uint8_t*  allBytes_    = nullptr;
    size_t          allBytesLen_ = 0;

    // Owned storage when built in memory (empty when mmap'd).
    std::vector<uint64_t> ownMasks_, ownBnMasks_, ownBnBoundaries_;
    std::vector<uint32_t> ownByteOffsets_;
    std::vector<uint16_t> ownByteLengths_, ownBnStarts_, ownExtIds_;
    std::vector<uint8_t>  ownSegCounts_, ownIsDirs_, ownAllBytes_;

    // Extension string table, parallel to extension ids used at build time.
    std::vector<std::string> extTable_;

    // mmap bookkeeping.
    void*  mmapBase_ = nullptr;
    size_t mmapLen_  = 0;
};

} // namespace mff

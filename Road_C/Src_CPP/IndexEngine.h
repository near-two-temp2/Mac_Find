// IndexEngine.h — self-built binary index (bitmask prefilter + fzf), Road_C primary.
//
// Design follows Cling (see ../../open-source-analysis.md §3):
//   * Build:  walk the filesystem with fts(3), lowercase each path, store it as
//             packed bytes plus parallel per-entry arrays.
//   * Prefilter (Phase 1): a 64-bit letter/digit/punct bitmask per path lets us
//             reject non-candidates with a single AND — O(1) per entry.
//   * Score   (Phase 2): fzf-style greedy fuzzy match on the survivors, with
//             word-boundary and contiguity bonuses, sorted best-first.
//
// The index serialises to a mmap-friendly .idx file guarded by a magic number so
// the hybrid coordinator can detect a missing/corrupt index and fall back to
// searchfs().
#pragma once

#include "SearchTypes.h"

#include <cstdint>
#include <functional>
#include <string>
#include <vector>

namespace macfind {

// Magic at the head of every .idx: the ASCII bytes "MFCX_IX1". Any mismatch on
// load means the file isn't ours or is truncated → treated as corrupt.
constexpr uint64_t kIndexMagic = 0x3158495F5843464DULL; // "MFCX_IX1" (LE)

// Default location of the on-disk index.
std::string defaultIndexPath();

// Default roots to index when the caller doesn't specify any: the user's home
// plus /Applications (mirrors the Road_C Tauri reference). Network/FUSE mounts
// under these are pruned during the walk (see VolumeFilter).
std::vector<std::string> defaultIndexRoots();

// Compute the 64-bit character bitmask of a lowercased string (a-z, 0-9, . - _).
uint64_t charMask(const std::string& lowered);

// fzf-style fuzzy score of `pattern` (already lowercased) against `text`
// (already lowercased). Returns a score; higher is better. `matched` is false
// when the pattern isn't even a subsequence.
struct FuzzyScore {
    bool matched = false;
    int  score   = 0;
};
FuzzyScore fuzzyScore(const std::string& pattern, const std::string& text,
                      std::size_t boundaryHintStart);

// Rank a query against one indexed path, combining coarse match *tiers* (exact
// basename > prefix > substring > scattered subsequence) with the fine-grained
// fzf score inside each tier. This is what puts `/Users/oracle/temp_test` at
// the top for the query `temp_test` instead of letting scattered fzf noise like
// `vscode_pytest` compete. `matched` is false when the pattern isn't even a
// subsequence of the path (i.e. not a real hit).
//   `lowPath`     — the lowercased full path.
//   `lowPattern`  — the lowercased query.
//   `bnStart`     — offset of the basename within `lowPath`.
struct RankScore {
    bool matched = false;
    long score   = 0;   // large dynamic range → tier * base + fzf refinement
};
RankScore rankPath(const std::string& lowPattern, const std::string& lowPath,
                   std::size_t bnStart);

class IndexEngine {
public:
    // --- Build / persistence ---

    // Walk `roots` (defaults to $HOME) and build the in-memory index.
    // `progress`, if set, is called periodically with the running entry count.
    bool build(const std::vector<std::string>& roots = {},
               const std::function<void(std::size_t)>& progress = nullptr);

    // Serialise the current in-memory index to `path`.
    bool save(const std::string& path) const;

    // Load an index from `path`. Returns false if the file is missing, too
    // short, or its magic/counts don't line up (treated as "corrupt").
    bool load(const std::string& path);

    bool loaded() const { return loaded_; }
    std::size_t entryCount() const { return byteOffsets_.size(); }

    // --- Query ---

    // Two-phase query: bitmask prefilter, then fzf scoring on survivors.
    // Results are sorted by score (best first) and truncated to opts.limit.
    SearchOutcome query(const std::string& term, const SearchOptions& opts) const;

private:
    void addEntry(const std::string& path, bool isDir);

    // Parallel arrays, one slot per indexed path (Cling layout, simplified).
    // We keep both the original-case bytes (for display + case-sensitive match +
    // Reveal in Finder) and a lowercased copy (for the cheap fuzzy match). Both
    // share the same offsets/lengths since lowercasing is 1:1 on bytes.
    std::vector<uint8_t>  allBytes_;      // packed original-case UTF-8 of every path
    std::vector<uint8_t>  lowBytes_;      // packed lowercase UTF-8 (match target)
    std::vector<uint32_t> byteOffsets_;   // start of path i within *both* pools
    std::vector<uint16_t> byteLengths_;   // length of path i
    std::vector<uint16_t> bnStarts_;      // basename offset within path i
    std::vector<uint64_t> masks_;         // char bitmask of the whole path
    std::vector<uint64_t> bnMasks_;       // char bitmask of the basename
    std::vector<uint8_t>  isDirs_;        // 1 if entry is a directory

    bool loaded_ = false;
};

}  // namespace macfind

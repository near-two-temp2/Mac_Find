// IndexEngine.cpp — self-built binary index: fts scan, bitmask prefilter, fzf.
#include "IndexEngine.h"
#include "VolumeFilter.h"

#include <algorithm>
#include <cctype>
#include <climits>
#include <cstdio>
#include <cstring>
#include <fstream>

#include <fts.h>
#include <unistd.h>
#include <pwd.h>
#include <sys/stat.h>

namespace macfind {

namespace {

// Lowercase a byte the ASCII way (paths are stored lowercased for cheap matching).
inline char lc(char c) {
    return (c >= 'A' && c <= 'Z') ? static_cast<char>(c - 'A' + 'a') : c;
}

std::string lowered(const std::string& s) {
    std::string out(s.size(), '\0');
    for (std::size_t i = 0; i < s.size(); ++i) out[i] = lc(s[i]);
    return out;
}

// Bit slot for a lowercased char: a-z → 0..25, 0-9 → 26..35, '.'/'-'/'_' → 36..38.
inline int bitSlot(char c) {
    if (c >= 'a' && c <= 'z') return c - 'a';
    if (c >= '0' && c <= '9') return 26 + (c - '0');
    if (c == '.') return 36;
    if (c == '-') return 37;
    if (c == '_') return 38;
    return -1;  // other bytes don't participate in the prefilter
}

std::string homeDir() {
    if (const char* h = getenv("HOME")) return h;
    if (const passwd* pw = getpwuid(getuid())) return pw->pw_dir;
    return "/";
}

// Is byte at index i a word boundary start (preceded by separator or camelCase)?
bool isBoundary(const std::string& text, std::size_t i) {
    if (i == 0) return true;
    char prev = text[i - 1];
    return prev == '/' || prev == '_' || prev == '-' || prev == '.' || prev == ' ';
}

}  // namespace

std::string defaultIndexPath() {
    // ~/Library/Caches/org.macfind.roadc.cpp/index.idx (mirrors Cling's location).
    return homeDir() + "/Library/Caches/org.macfind.roadc.cpp.idx";
}

std::vector<std::string> defaultIndexRoots() {
    std::vector<std::string> roots{homeDir()};
    if (access("/Applications", F_OK) == 0) roots.emplace_back("/Applications");
    return roots;
}

uint64_t charMask(const std::string& low) {
    uint64_t m = 0;
    for (char c : low) {
        int s = bitSlot(c);
        if (s >= 0) m |= (uint64_t{1} << s);
    }
    return m;
}

FuzzyScore fuzzyScore(const std::string& pattern, const std::string& text,
                      std::size_t boundaryHintStart) {
    FuzzyScore fs;
    if (pattern.empty()) { fs.matched = true; fs.score = 1; return fs; }

    // Scoring weights (mirroring Cling's fzf tuning at a smaller scale).
    constexpr int kMatch      = 16;   // per matched char
    constexpr int kConsec     = 4;    // adjacent to previous match
    constexpr int kBoundary   = 8;    // match lands on a word boundary
    constexpr int kFirstBonus = 8;    // first pattern char at basename start
    constexpr int kGapStart   = -3;
    constexpr int kGapExtend  = -1;

    int         score      = 0;
    std::size_t pi         = 0;       // pattern cursor
    std::size_t lastMatch  = std::string::npos;
    bool        inGap       = false;

    for (std::size_t ti = 0; ti < text.size() && pi < pattern.size(); ++ti) {
        if (text[ti] == pattern[pi]) {
            score += kMatch;
            if (isBoundary(text, ti)) score += kBoundary;
            if (pi == 0 && ti >= boundaryHintStart) score += kFirstBonus;
            if (lastMatch != std::string::npos && ti == lastMatch + 1) score += kConsec;
            lastMatch = ti;
            inGap = false;
            ++pi;
        } else if (lastMatch != std::string::npos) {
            score += inGap ? kGapExtend : kGapStart;
            inGap = true;
        }
    }

    fs.matched = (pi == pattern.size());  // whole pattern consumed as subsequence
    fs.score   = fs.matched ? std::max(score, 1) : 0;
    return fs;
}

RankScore rankPath(const std::string& pat, const std::string& low,
                   std::size_t bnStart) {
    RankScore rs;
    if (pat.empty()) { rs.matched = true; rs.score = 1; return rs; }

    // Coarse tiers dominate the ordering; the fzf score only breaks ties
    // *within* a tier. The gap between tiers (kTier) is far larger than any
    // realistic fzf score for a short query, so an exact/substring hit can
    // never be overtaken by scattered subsequence noise. This is the fix for
    // "temp_test → /Users/oracle/temp_test ranks #1" (SEARCH_TEST_BASELINE.md).
    constexpr long kTier = 1'000'000;   // >> any fzf score
    enum Tier {
        T_NONE          = 0,
        T_SUBSEQ        = 1,  // scattered fzf subsequence (weakest real hit)
        T_PATH_SUBSTR   = 2,  // contiguous substring somewhere in the path
        T_BN_SUBSTR     = 3,  // contiguous substring inside the basename
        T_BN_PREFIX     = 4,  // basename starts with the query
        T_BN_EXACT      = 5,  // basename == query (the bullseye)
    };

    const std::string bn = low.substr(bnStart);

    int tier = T_NONE;
    if (bn == pat) {
        tier = T_BN_EXACT;
    } else if (bn.size() >= pat.size() && bn.compare(0, pat.size(), pat) == 0) {
        tier = T_BN_PREFIX;
    } else if (bn.find(pat) != std::string::npos) {
        tier = T_BN_SUBSTR;
    } else if (low.find(pat) != std::string::npos) {
        tier = T_PATH_SUBSTR;
    }

    // Fine score: fzf over the whole path (biased to the basename start). Used
    // both to rank inside a tier and to decide subsequence membership for the
    // weakest tier.
    FuzzyScore fzf = fuzzyScore(pat, low, bnStart);
    if (tier == T_NONE) {
        // No contiguous hit anywhere; fall back to scattered subsequence only.
        if (!fzf.matched) { rs.matched = false; return rs; }
        tier = T_SUBSEQ;
    }

    // Shorter paths win ties (more specific / shallower). Encode as a small
    // negative nudge so it never crosses a tier or overwhelms fzf ranking.
    const long shallow = -static_cast<long>(std::min<std::size_t>(low.size(), 4096));

    rs.matched = true;
    rs.score   = static_cast<long>(tier) * kTier
               + static_cast<long>(fzf.score) * 8
               + shallow;
    return rs;
}

void IndexEngine::addEntry(const std::string& path, bool isDir) {
    if (path.size() > UINT16_MAX) return;  // skip absurdly long paths
    std::string low = lowered(path);

    uint32_t off = static_cast<uint32_t>(allBytes_.size());
    allBytes_.insert(allBytes_.end(), path.begin(), path.end());
    lowBytes_.insert(lowBytes_.end(), low.begin(), low.end());

    auto slash = low.find_last_of('/');
    uint16_t bnStart = (slash == std::string::npos)
                           ? 0
                           : static_cast<uint16_t>(slash + 1);

    byteOffsets_.push_back(off);
    byteLengths_.push_back(static_cast<uint16_t>(path.size()));
    bnStarts_.push_back(bnStart);
    masks_.push_back(charMask(low));
    bnMasks_.push_back(charMask(low.substr(bnStart)));
    isDirs_.push_back(isDir ? 1 : 0);
}

bool IndexEngine::build(const std::vector<std::string>& roots,
                        const std::function<void(std::size_t)>& progress) {
    allBytes_.clear();
    lowBytes_.clear();
    byteOffsets_.clear();
    byteLengths_.clear();
    bnStarts_.clear();
    masks_.clear();
    bnMasks_.clear();
    isDirs_.clear();

    std::vector<std::string> scanRoots = roots.empty()
                                             ? defaultIndexRoots()
                                             : roots;

    for (const auto& root : scanRoots) {
        // Never even open a root that's already a known network/FUSE mount.
        if (isExplicitlyExcluded(root)) continue;

        char* const paths[] = {const_cast<char*>(root.c_str()), nullptr};
        // We keep FTS_PHYSICAL (don't follow symlinks) but drop FTS_NOSTAT: we
        // need fts_statp->st_dev on directories to detect when the walk crosses
        // a mount boundary. The stat cost is paid only on real inodes fts must
        // visit anyway; it's worth it to guarantee we never wander onto a
        // network/FUSE mount (rclone→B2), which would be slow and cost money.
        FTS* fts = fts_open(paths, FTS_PHYSICAL | FTS_NOCHDIR, nullptr);
        if (!fts) continue;

        // Device id of the root's own volume. Crossing onto a *different*
        // device means we hit a submount; we re-admit it only if statfs says
        // it's a local apfs/hfs volume (mountIsIndexable) — otherwise prune.
        dev_t rootDev = 0;
        bool  haveRootDev = false;

        FTSENT* ent;
        while ((ent = fts_read(fts)) != nullptr) {
            switch (ent->fts_info) {
                case FTS_D: {  // directory, pre-order
                    const dev_t dev = ent->fts_statp ? ent->fts_statp->st_dev : 0;
                    if (!haveRootDev) { rootDev = dev; haveRootDev = true; }

                    // Belt: explicit blocklist (h2-* rclone, CloudStorage).
                    if (isExplicitlyExcluded(ent->fts_path)) {
                        fts_set(fts, ent, FTS_SKIP);  // don't descend
                        break;
                    }
                    // Suspenders: any directory on a device other than the
                    // root's is a crossed mount — only descend if it's a local
                    // catalog volume. This prunes network mounts by device.
                    if (dev != rootDev && !mountIsIndexable(ent->fts_path)) {
                        fts_set(fts, ent, FTS_SKIP);  // prune the whole subtree
                        break;
                    }
                    addEntry(ent->fts_path, /*isDir=*/true);
                    break;
                }
                case FTS_DP:   // directory, post-order — already counted at FTS_D
                    break;
                case FTS_DNR:  // directory we can't read — still index its name
                    addEntry(ent->fts_path, /*isDir=*/true);
                    break;
                case FTS_F:      // regular file
                case FTS_SL:     // symlink
                case FTS_SLNONE: // broken symlink
                case FTS_NSOK:   // stat unavailable — index the name anyway
                case FTS_DEFAULT:
                    addEntry(ent->fts_path, /*isDir=*/false);
                    break;
                default:
                    break;
            }
            if (progress && (byteOffsets_.size() & 0x3FFF) == 0) {
                progress(byteOffsets_.size());
            }
        }
        fts_close(fts);
    }

    loaded_ = true;
    if (progress) progress(byteOffsets_.size());
    return true;
}

bool IndexEngine::save(const std::string& path) const {
    std::ofstream f(path, std::ios::binary | std::ios::trunc);
    if (!f) return false;

    auto put64 = [&](uint64_t v) { f.write(reinterpret_cast<const char*>(&v), 8); };
    const uint64_t n     = byteOffsets_.size();
    const uint64_t bytes = allBytes_.size();

    // Header: magic, entry count, allBytes byte count.
    put64(kIndexMagic);
    put64(n);
    put64(bytes);

    // Parallel arrays, then the two bulk byte pools — mmap-friendly ordering.
    f.write(reinterpret_cast<const char*>(byteOffsets_.data()), n * sizeof(uint32_t));
    f.write(reinterpret_cast<const char*>(byteLengths_.data()), n * sizeof(uint16_t));
    f.write(reinterpret_cast<const char*>(bnStarts_.data()),    n * sizeof(uint16_t));
    f.write(reinterpret_cast<const char*>(masks_.data()),       n * sizeof(uint64_t));
    f.write(reinterpret_cast<const char*>(bnMasks_.data()),     n * sizeof(uint64_t));
    f.write(reinterpret_cast<const char*>(isDirs_.data()),      n * sizeof(uint8_t));
    f.write(reinterpret_cast<const char*>(allBytes_.data()),    bytes);
    f.write(reinterpret_cast<const char*>(lowBytes_.data()),    bytes);

    return static_cast<bool>(f);
}

bool IndexEngine::load(const std::string& path) {
    loaded_ = false;
    std::ifstream f(path, std::ios::binary);
    if (!f) return false;

    auto get64 = [&](uint64_t& v) -> bool {
        return static_cast<bool>(f.read(reinterpret_cast<char*>(&v), 8));
    };

    uint64_t magic = 0, n = 0, bytes = 0;
    if (!get64(magic) || magic != kIndexMagic) return false;  // corrupt / not ours
    if (!get64(n) || !get64(bytes)) return false;

    // Guard against absurd sizes that would blow memory on a truncated file.
    if (n > (1ull << 34) || bytes > (1ull << 36)) return false;

    byteOffsets_.resize(n);
    byteLengths_.resize(n);
    bnStarts_.resize(n);
    masks_.resize(n);
    bnMasks_.resize(n);
    isDirs_.resize(n);
    allBytes_.resize(bytes);
    lowBytes_.resize(bytes);

    auto readInto = [&](void* dst, std::size_t sz) -> bool {
        return sz == 0 || static_cast<bool>(f.read(reinterpret_cast<char*>(dst), sz));
    };

    if (!readInto(byteOffsets_.data(), n * sizeof(uint32_t)) ||
        !readInto(byteLengths_.data(), n * sizeof(uint16_t)) ||
        !readInto(bnStarts_.data(),    n * sizeof(uint16_t)) ||
        !readInto(masks_.data(),       n * sizeof(uint64_t)) ||
        !readInto(bnMasks_.data(),     n * sizeof(uint64_t)) ||
        !readInto(isDirs_.data(),      n * sizeof(uint8_t))  ||
        !readInto(allBytes_.data(),    bytes)                ||
        !readInto(lowBytes_.data(),    bytes)) {
        return false;  // truncated payload → corrupt
    }

    loaded_ = true;
    return true;
}

SearchOutcome IndexEngine::query(const std::string& term,
                                 const SearchOptions& opts) const {
    SearchOutcome out;
    out.backend = Backend::Index;

    if (!loaded_) {
        out.ok = false;
        out.error = "Index not loaded.";
        return out;
    }
    if (term.empty()) return out;  // empty query → empty (fast) result set

    const std::string pat  = lowered(term);
    const uint64_t    want = charMask(pat);

    // Phase 1 + 2 combined in a single pass over the parallel arrays.
    struct Scored { uint32_t idx; long score; };
    std::vector<Scored> hits;

    const char* lowPool  = reinterpret_cast<const char*>(lowBytes_.data());
    const char* origPool = reinterpret_cast<const char*>(allBytes_.data());
    const std::size_t n = byteOffsets_.size();

    for (std::size_t i = 0; i < n; ++i) {
        // Cheap dir/file gate first.
        if (opts.dirsOnly  && !isDirs_[i]) continue;
        if (opts.filesOnly &&  isDirs_[i]) continue;

        // Phase 1: one AND rejects any path missing a required char class.
        if ((masks_[i] & want) != want) continue;

        // Phase 2: tiered rank (exact/prefix/substring > scattered fzf) so the
        // literal `temp_test` directory beats fzf noise; see rankPath().
        std::string low(lowPool + byteOffsets_[i], byteLengths_[i]);
        RankScore rscore = rankPath(pat, low, bnStarts_[i]);
        if (!rscore.matched) continue;

        // Case-sensitive mode: require the original term as a literal substring
        // of the original-case path (the rank already confirmed a lowercased
        // hit, so this only tightens, never loosens).
        if (opts.caseSensitive) {
            std::string orig(origPool + byteOffsets_[i], byteLengths_[i]);
            if (orig.find(term) == std::string::npos) continue;
        }

        hits.push_back({static_cast<uint32_t>(i), rscore.score});
    }

    // Best-first by tiered rank score.
    std::sort(hits.begin(), hits.end(),
              [](const Scored& a, const Scored& b) { return a.score > b.score; });

    std::size_t take = hits.size();
    if (opts.limit && opts.limit < take) take = opts.limit;
    out.results.reserve(take);
    for (std::size_t k = 0; k < take; ++k) {
        const auto& h = hits[k];
        SearchResult r;
        r.path  = std::string(origPool + byteOffsets_[h.idx], byteLengths_[h.idx]);
        r.isDir = isDirs_[h.idx] != 0;
        r.score = static_cast<int>(h.score);  // tiered score fits in int (< 2^31)
        out.results.push_back(std::move(r));
    }
    return out;
}

}  // namespace macfind

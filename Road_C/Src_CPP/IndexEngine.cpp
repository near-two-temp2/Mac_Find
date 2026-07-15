// IndexEngine.cpp — self-built binary index: fts scan, bitmask prefilter, fzf.
#include "IndexEngine.h"

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
                                             ? std::vector<std::string>{homeDir()}
                                             : roots;

    for (const auto& root : scanRoots) {
        char* const paths[] = {const_cast<char*>(root.c_str()), nullptr};
        // FTS_NOSTAT keeps the walk fast (Cling relies on the same trick). The
        // catch: with FTS_NOSTAT, non-directory entries arrive as FTS_NSOK
        // (stat not requested) rather than FTS_F, so we must handle that too —
        // otherwise only directories get indexed. fts still stats directories
        // (it has to, to recurse), so FTS_D remains reliable for the isDir flag.
        FTS* fts = fts_open(paths, FTS_PHYSICAL | FTS_NOSTAT | FTS_NOCHDIR, nullptr);
        if (!fts) continue;

        FTSENT* ent;
        while ((ent = fts_read(fts)) != nullptr) {
            switch (ent->fts_info) {
                case FTS_D:    // directory, pre-order
                    addEntry(ent->fts_path, /*isDir=*/true);
                    break;
                case FTS_DP:   // directory, post-order — already counted at FTS_D
                    break;
                case FTS_DNR:  // directory we can't read — still index its name
                    addEntry(ent->fts_path, /*isDir=*/true);
                    break;
                case FTS_F:      // regular file (only when stat info is present)
                case FTS_SL:     // symlink
                case FTS_SLNONE: // broken symlink
                case FTS_NSOK:   // no stat requested — the common case under FTS_NOSTAT
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
    struct Scored { uint32_t idx; int score; };
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

        // Phase 2: fzf score against the lowercased path, biased to the basename.
        std::string low(lowPool + byteOffsets_[i], byteLengths_[i]);
        FuzzyScore fscore = fuzzyScore(pat, low, bnStarts_[i]);
        if (!fscore.matched) continue;

        // Case-sensitive mode: require the original term as a literal substring
        // of the original-case path (fzf already confirmed the lowercased
        // subsequence, so this only tightens, never loosens).
        if (opts.caseSensitive) {
            std::string orig(origPool + byteOffsets_[i], byteLengths_[i]);
            if (orig.find(term) == std::string::npos) continue;
        }

        hits.push_back({static_cast<uint32_t>(i), fscore.score});
    }

    // Best-first by fzf score.
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
        r.score = h.score;
        out.results.push_back(std::move(r));
    }
    return out;
}

}  // namespace macfind

#include "index_engine.hpp"

#include "fzf.hpp"
#include "scanner.hpp"

#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>

#include <algorithm>
#include <cstdio>
#include <cstring>
#include <thread>
#include <unordered_map>

namespace mff {

// ---- small helpers -------------------------------------------------------

// Lowercase ASCII in place. Paths are treated as bytes; non-ASCII passes
// through unchanged (bitmask simply ignores those bytes).
static inline uint8_t lc(uint8_t c) {
    return (c >= 'A' && c <= 'Z') ? uint8_t(c + 32) : c;
}

// Locate the basename start (index just after the last '/').
static uint16_t basenameStart(const uint8_t* p, uint16_t len) {
    uint16_t start = 0;
    for (uint16_t i = 0; i < len; ++i)
        if (p[i] == '/') start = uint16_t(i + 1);
    return start;
}

// Word-boundary bitmap for the first 64 bytes of the basename: bit i set when
// basename byte i begins a word (position 0, or preceded by a separator).
static uint64_t basenameBoundaries(const uint8_t* p, uint16_t len,
                                   uint16_t bnStart) {
    uint64_t bits = 0;
    uint16_t n = uint16_t(std::min<int>(len - bnStart, 64));
    for (uint16_t i = 0; i < n; ++i) {
        uint16_t idx = bnStart + i;
        bool boundary = (i == 0);
        if (!boundary) {
            uint8_t prev = p[idx - 1];
            boundary = prev == '_' || prev == '-' || prev == '.' || prev == ' ';
        }
        if (boundary) bits |= (uint64_t(1) << i);
    }
    return bits;
}

// Extension (bytes after the last '.' in the basename), lowercased, no dot.
static std::string extensionOf(const uint8_t* p, uint16_t len, uint16_t bnStart) {
    int dot = -1;
    for (uint16_t i = bnStart; i < len; ++i)
        if (p[i] == '.') dot = i;
    if (dot < 0 || dot + 1 >= len) return {};
    std::string e;
    for (int i = dot + 1; i < len; ++i) e.push_back(char(lc(p[i])));
    return e;
}

// ---- lifecycle -----------------------------------------------------------

IndexEngine::~IndexEngine() { reset(); }

void IndexEngine::reset() {
    if (mmapBase_) {
        munmap(mmapBase_, mmapLen_);
        mmapBase_ = nullptr;
        mmapLen_ = 0;
    }
    ownMasks_.clear(); ownBnMasks_.clear(); ownBnBoundaries_.clear();
    ownByteOffsets_.clear(); ownByteLengths_.clear(); ownBnStarts_.clear();
    ownExtIds_.clear(); ownSegCounts_.clear(); ownIsDirs_.clear();
    ownAllBytes_.clear(); extTable_.clear();
    entryCount_ = 0; allBytesLen_ = 0;
    masks_ = bnMasks_ = bnBoundaries_ = nullptr;
    byteOffsets_ = nullptr; byteLengths_ = bnStarts_ = extIds_ = nullptr;
    segCounts_ = isDirs_ = allBytes_ = nullptr;
}

// ---- build ---------------------------------------------------------------

size_t IndexEngine::buildFromRoots(const std::vector<std::string>& roots) {
    reset();
    std::unordered_map<std::string, uint16_t> extIds;
    extTable_.emplace_back(""); // id 0 = no extension

    auto sink = [&](const std::string& rawPath, bool isDir) {
        size_t len = rawPath.size();
        if (len == 0 || len > 65535) return; // byteLengths_ is UInt16

        uint32_t offset = uint32_t(ownAllBytes_.size());
        // Append lowercased path bytes to the blob.
        for (char c : rawPath) ownAllBytes_.push_back(lc(uint8_t(c)));
        const uint8_t* p = ownAllBytes_.data() + offset;
        uint16_t l = uint16_t(len);

        uint16_t bnStart = basenameStart(p, l);
        uint64_t mask   = computeMask(p, l);
        uint64_t bnMask = computeMask(p + bnStart, uint16_t(l - bnStart));
        uint64_t bnB    = basenameBoundaries(p, l, bnStart);

        uint8_t segs = 0;
        for (uint16_t i = 0; i < l; ++i) if (p[i] == '/') segs = uint8_t(std::min(segs + 1, 255));

        std::string ext = extensionOf(p, l, bnStart);
        uint16_t extId = 0;
        if (!ext.empty()) {
            auto it = extIds.find(ext);
            if (it != extIds.end()) extId = it->second;
            else if (extTable_.size() < 65535) {
                extId = uint16_t(extTable_.size());
                extIds.emplace(ext, extId);
                extTable_.push_back(ext);
            }
        }

        ownMasks_.push_back(mask);
        ownBnMasks_.push_back(bnMask);
        ownBnBoundaries_.push_back(bnB);
        ownByteOffsets_.push_back(offset);
        ownByteLengths_.push_back(l);
        ownBnStarts_.push_back(bnStart);
        ownExtIds_.push_back(extId);
        ownSegCounts_.push_back(segs);
        ownIsDirs_.push_back(isDir ? 1 : 0);
    };

    scanRoots(roots, sink);

    // Point the search-time pointers at the owned vectors.
    entryCount_   = ownIsDirs_.size();
    allBytesLen_  = ownAllBytes_.size();
    masks_        = ownMasks_.data();
    bnMasks_      = ownBnMasks_.data();
    bnBoundaries_ = ownBnBoundaries_.data();
    byteOffsets_  = ownByteOffsets_.data();
    byteLengths_  = ownByteLengths_.data();
    bnStarts_     = ownBnStarts_.data();
    extIds_       = ownExtIds_.data();
    segCounts_    = ownSegCounts_.data();
    isDirs_       = ownIsDirs_.data();
    allBytes_     = ownAllBytes_.data();
    return entryCount_;
}

// ---- persistence ---------------------------------------------------------

// Append `n` bytes from `src` to `out`.
static void put(std::string& out, const void* src, size_t n) {
    out.append(reinterpret_cast<const char*>(src), n);
}

bool IndexEngine::save(const std::string& path) const {
    if (entryCount_ == 0 && allBytesLen_ == 0) {
        // Still write a valid empty index so callers/CI get a file.
    }

    IndexHeader h;
    h.magic       = kIndexMagic;
    h.entryCount  = entryCount_;
    h.allBytesLen = allBytesLen_;
    h.reserved    = 0;

    std::string buf;
    buf.reserve(sizeof(h) +
                entryCount_ * (8 + 8 + 8 + 4 + 2 + 2 + 2 + 1 + 1) +
                allBytesLen_);
    put(buf, &h, sizeof(h));
    const size_t n = entryCount_;
    put(buf, masks_,        n * sizeof(uint64_t));
    put(buf, bnMasks_,      n * sizeof(uint64_t));
    put(buf, bnBoundaries_, n * sizeof(uint64_t));
    put(buf, byteOffsets_,  n * sizeof(uint32_t));
    put(buf, byteLengths_,  n * sizeof(uint16_t));
    put(buf, bnStarts_,     n * sizeof(uint16_t));
    put(buf, extIds_,       n * sizeof(uint16_t));
    put(buf, segCounts_,    n * sizeof(uint8_t));
    put(buf, isDirs_,       n * sizeof(uint8_t));
    put(buf, allBytes_,     allBytesLen_);

    FILE* f = std::fopen(path.c_str(), "wb");
    if (!f) return false;
    size_t wrote = std::fwrite(buf.data(), 1, buf.size(), f);
    std::fclose(f);
    return wrote == buf.size();
}

bool IndexEngine::loadMmap(const std::string& path) {
    reset();
    int fd = ::open(path.c_str(), O_RDONLY);
    if (fd < 0) return false;
    struct stat st{};
    if (fstat(fd, &st) != 0 || st.st_size < (off_t)sizeof(IndexHeader)) {
        ::close(fd);
        return false;
    }
    size_t fileLen = (size_t)st.st_size;
    void* base = mmap(nullptr, fileLen, PROT_READ, MAP_PRIVATE, fd, 0);
    ::close(fd);
    if (base == MAP_FAILED) return false;

    const uint8_t* cur = reinterpret_cast<const uint8_t*>(base);
    IndexHeader h;
    std::memcpy(&h, cur, sizeof(h));
    if (h.magic != kIndexMagic) { munmap(base, fileLen); return false; }

    const size_t n = h.entryCount;
    const size_t need = sizeof(IndexHeader) +
                        n * (8 + 8 + 8 + 4 + 2 + 2 + 2 + 1 + 1) + h.allBytesLen;
    if (need > fileLen) { munmap(base, fileLen); return false; }

    mmapBase_ = base;
    mmapLen_  = fileLen;
    entryCount_  = n;
    allBytesLen_ = h.allBytesLen;

    // Carve the parallel arrays out of the mapping in the same order as save().
    cur += sizeof(IndexHeader);
    auto take = [&](size_t bytes) -> const uint8_t* {
        const uint8_t* p = cur; cur += bytes; return p;
    };
    masks_        = reinterpret_cast<const uint64_t*>(take(n * 8));
    bnMasks_      = reinterpret_cast<const uint64_t*>(take(n * 8));
    bnBoundaries_ = reinterpret_cast<const uint64_t*>(take(n * 8));
    byteOffsets_  = reinterpret_cast<const uint32_t*>(take(n * 4));
    byteLengths_  = reinterpret_cast<const uint16_t*>(take(n * 2));
    bnStarts_     = reinterpret_cast<const uint16_t*>(take(n * 2));
    extIds_       = reinterpret_cast<const uint16_t*>(take(n * 2));
    segCounts_    = take(n * 1);
    isDirs_       = take(n * 1);
    allBytes_     = take(allBytesLen_);
    return true;
}

// ---- search --------------------------------------------------------------

std::vector<SearchHit> IndexEngine::search(const std::string& query,
                                           const SearchOptions& opts) const {
    std::vector<SearchHit> out;
    if (entryCount_ == 0) return out;

    // Lowercase the query and compute its bitmask once.
    std::vector<uint8_t> q;
    q.reserve(query.size());
    for (char c : query) q.push_back(lc(uint8_t(c)));
    uint64_t qMask = computeMask(q.data(), (uint32_t)q.size());

    std::string extLc;
    for (char c : opts.extension) extLc.push_back(char(lc(uint8_t(c))));

    // Per-thread result buckets to avoid locking during the hot scan.
    unsigned hw = std::thread::hardware_concurrency();
    unsigned nThreads = std::max(1u, std::min(hw ? hw : 1u, 8u));
    std::vector<std::vector<SearchHit>> buckets(nThreads);

    auto worker = [&](unsigned tid) {
        std::vector<SearchHit>& local = buckets[tid];
        size_t begin = (entryCount_ * tid) / nThreads;
        size_t end   = (entryCount_ * (tid + 1)) / nThreads;
        for (size_t i = begin; i < end; ++i) {
            // Phase 1a: type filter.
            bool isDir = isDirs_[i] != 0;
            if (opts.filesOnly && isDir) continue;
            if (opts.dirsOnly && !isDir) continue;

            // Phase 1b: bitmask prefilter — every query letter must be present
            // somewhere in the path. One UInt64 AND rejects most candidates.
            if ((masks_[i] & qMask) != qMask) continue;

            const uint8_t* p = pathBytes(i);
            uint16_t len = byteLengths_[i];
            uint16_t bnStart = bnStarts_[i];

            // Phase 1c: extension filter (derived from bytes so it works on
            // mmap'd indexes with no string table).
            if (!extLc.empty()) {
                std::string e = extensionOf(p, len, bnStart);
                if (e != extLc) continue;
            }

            // Phase 2: fzf score. Prefer matching against the basename (bonus
            // boundaries precomputed); fall back to whole path.
            const uint8_t* bn = p + bnStart;
            uint16_t bnLen = uint16_t(len - bnStart);
            ScoreResult r = fuzzyScore(q.data(), q.size(), bn, bnLen,
                                       bnBoundaries_[i], true);
            int score;
            if (r.matched) {
                score = r.score + 20; // basename match bonus
            } else {
                ScoreResult rp = fuzzyScore(q.data(), q.size(), p, len);
                if (!rp.matched) continue;
                score = rp.score;
            }

            local.push_back(SearchHit{
                std::string(reinterpret_cast<const char*>(p), len),
                score, isDir});
        }
    };

    std::vector<std::thread> pool;
    for (unsigned t = 1; t < nThreads; ++t) pool.emplace_back(worker, t);
    worker(0);
    for (auto& th : pool) th.join();

    // Merge buckets.
    size_t total = 0;
    for (auto& b : buckets) total += b.size();
    out.reserve(total);
    for (auto& b : buckets)
        out.insert(out.end(), b.begin(), b.end());

    // Rank by score desc, then shorter path first (tie-break toward tighter
    // matches), then lexicographic for stability.
    std::sort(out.begin(), out.end(), [](const SearchHit& a, const SearchHit& b) {
        if (a.score != b.score) return a.score > b.score;
        if (a.path.size() != b.path.size()) return a.path.size() < b.path.size();
        return a.path < b.path;
    });
    if ((int)out.size() > opts.maxResults) out.resize(opts.maxResults);
    return out;
}

} // namespace mff

// index_format.hpp — on-disk binary index layout for Road_B (C++).
//
// Design mirrors Cling's parallel-array / mmap-friendly index (see
// open-source-analysis.md §3.3). Every field lives in its own contiguous
// array so that Phase-1 filtering scans one tight array at a time and the
// whole file can be mmap'd and used in place with zero parsing.
//
//   Header (fixed size)
//   masks[]       UInt64  path letter bitmask        (Phase-1 prefilter)
//   bnMasks[]     UInt64  basename letter bitmask     (Phase-1 prefilter)
//   bnBoundaries[]UInt64  word-boundary bitmap of basename (fzf bonus)
//   byteOffsets[] UInt32  offset of path in allBytes
//   byteLengths[] UInt16  path byte length
//   bnStarts[]    UInt16  basename start within the path
//   extIds[]      UInt16  extension id (Phase-1 prefilter)
//   segCounts[]   UInt8   number of '/' segments (depth filter)
//   isDirs[]      UInt8   1 = directory, 0 = file
//   allBytes[]    packed lowercase UTF-8 path bytes
//
// The file is written little-endian; macOS on Intel and Apple Silicon are
// both little-endian so we store native and do not byte-swap.

#pragma once

#include <cstdint>

namespace mff {

// "MFFBIDX1" as raw bytes, little-endian. Bump on any layout change.
static constexpr uint64_t kIndexMagic = 0x3158444942464653ULL;

// Bitmask bit assignments (matches Cling's scheme so the reasoning transfers).
//   bits 0-25  : letters a-z
//   bits 26-35 : digits 0-9
//   bit  36    : '.'
//   bit  37    : '-'
//   bit  38    : '_'
// Any other byte contributes no bit (it never disqualifies a candidate).

struct IndexHeader {
    uint64_t magic;       // kIndexMagic
    uint64_t entryCount;  // number of indexed paths
    uint64_t allBytesLen; // total bytes in the packed path blob
    uint64_t reserved;    // future use / alignment
};

// Map a raw lowercase byte to its bitmask bit index, or -1 if it sets no bit.
inline int maskBitForByte(uint8_t c) {
    if (c >= 'a' && c <= 'z') return c - 'a';        // 0-25
    if (c >= '0' && c <= '9') return 26 + (c - '0'); // 26-35
    if (c == '.') return 36;
    if (c == '-') return 37;
    if (c == '_') return 38;
    return -1;
}

// Compute the letter bitmask for a lowercase byte range.
inline uint64_t computeMask(const uint8_t* bytes, uint32_t len) {
    uint64_t m = 0;
    for (uint32_t i = 0; i < len; ++i) {
        int bit = maskBitForByte(bytes[i]);
        if (bit >= 0) m |= (uint64_t(1) << bit);
    }
    return m;
}

} // namespace mff

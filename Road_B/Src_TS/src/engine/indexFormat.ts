// indexFormat.ts — on-disk binary index layout for Road_B (TypeScript).
//
// Design mirrors Cling's parallel-array / mmap-friendly index (see
// open-source-analysis.md §3.3) and matches the C++ sibling
// (Road_B/Src_CPP/src/index_format.hpp) bit-for-bit so the reasoning
// transfers across implementations. Each field lives in its own contiguous
// typed array so Phase-1 filtering scans one tight array at a time.
//
//   Header (fixed size, 32 bytes)
//   masks[]        BigUint64  path letter bitmask       (Phase-1 prefilter)
//   bnMasks[]      BigUint64  basename letter bitmask    (Phase-1 prefilter)
//   bnBoundaries[] BigUint64  word-boundary bitmap of basename (fzf bonus)
//   byteOffsets[]  Uint32     offset of path in allBytes
//   byteLengths[]  Uint16     path byte length
//   bnStarts[]     Uint16     basename start within the path
//   extIds[]       Uint16     extension id (Phase-1 prefilter)
//   segCounts[]    Uint8      number of '/' segments (depth filter)
//   isDirs[]       Uint8      1 = directory, 0 = file
//   allBytes[]     packed lowercase UTF-8 path bytes
//
// Everything is little-endian; macOS on Intel and Apple Silicon are both
// little-endian, matching V8's typed-array native byte order.

// "MFFBIDX1" as raw bytes, little-endian. Bump on any layout change.
// Same value as the C++ sibling: 0x3158444942464653.
export const INDEX_MAGIC = 0x3158444942464653n;

// Header is 4 * 8 bytes.
export const HEADER_BYTES = 32;

// Bitmask bit assignments (identical to Cling / the C++ sibling):
//   bits 0-25  : letters a-z
//   bits 26-35 : digits 0-9
//   bit  36    : '.'
//   bit  37    : '-'
//   bit  38    : '_'
// Any other byte contributes no bit (it never disqualifies a candidate).

const CODE_a = 97;
const CODE_z = 122;
const CODE_0 = 48;
const CODE_9 = 57;
const CODE_DOT = 46;
const CODE_DASH = 45;
const CODE_USCORE = 95;

// Map a raw lowercase byte to its bitmask bit index, or -1 if it sets no bit.
export function maskBitForByte(c: number): number {
  if (c >= CODE_a && c <= CODE_z) return c - CODE_a; // 0-25
  if (c >= CODE_0 && c <= CODE_9) return 26 + (c - CODE_0); // 26-35
  if (c === CODE_DOT) return 36;
  if (c === CODE_DASH) return 37;
  if (c === CODE_USCORE) return 38;
  return -1;
}

// Compute the letter bitmask for a lowercase byte range [start, end).
export function computeMask(bytes: Uint8Array, start: number, end: number): bigint {
  let m = 0n;
  for (let i = start; i < end; i++) {
    const bit = maskBitForByte(bytes[i]);
    if (bit >= 0) m |= 1n << BigInt(bit);
  }
  return m;
}

// Word-boundary bitmap of the basename: a bit is set at position p (relative to
// basename start) when the byte at p begins a new "word". A word starts at
// index 0, after a separator ('.', '-', '_', ' ', '/'), or at a digit-run edge.
// Only the first 64 basename bytes are tracked (matches UInt64 width).
export function computeBoundaries(
  bytes: Uint8Array,
  bnStart: number,
  bnEnd: number
): bigint {
  let boundaries = 0n;
  const len = Math.min(bnEnd - bnStart, 64);
  let prevSep = true; // position 0 is always a boundary
  for (let i = 0; i < len; i++) {
    const c = bytes[bnStart + i];
    const isSep =
      c === CODE_DOT || c === CODE_DASH || c === CODE_USCORE || c === 32 || c === 47;
    if (prevSep && !isSep) {
      boundaries |= 1n << BigInt(i);
    }
    prevSep = isSep;
  }
  return boundaries;
}

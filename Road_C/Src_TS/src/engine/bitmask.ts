/*
 * bitmask.ts — 64-bit character presence mask, mirroring Cling's encoding
 * (see ../../../open-source-analysis.md §3.3):
 *
 *   Bits 0-25:  letters a-z
 *   Bits 26-35: digits 0-9
 *   Bit 36:     '.'  (dot)
 *   Bit 37:     '-'  (hyphen)
 *   Bit 38:     '_'  (underscore)
 *
 * The mask is used as an O(1) prefilter: if
 *   (entryMask & queryMask) !== queryMask
 * then the entry cannot contain every character class the query needs, so it
 * is skipped before any (more expensive) fuzzy scoring runs.
 *
 * JS numbers can't hold 64 bits precisely, so we carry the mask as a BigInt at
 * build time but store it into the index as two UInt32 words (lo/hi). At query
 * time we compare lo/hi words separately to stay in fast 32-bit integer land.
 */

// Returns the bit index (0..38) for a lowercase byte, or -1 if it isn't tracked.
function bitForCharCode(code: number): number {
  if (code >= 97 && code <= 122) return code - 97; // a-z -> 0..25
  if (code >= 48 && code <= 57) return 26 + (code - 48); // 0-9 -> 26..35
  if (code === 46) return 36; // '.'
  if (code === 45) return 37; // '-'
  if (code === 95) return 38; // '_'
  return -1;
}

/** Compute the {lo, hi} 32-bit words of the presence mask for a lowercase string. */
export function maskWords(lowerText: string): { lo: number; hi: number } {
  let lo = 0;
  let hi = 0;
  for (let i = 0; i < lowerText.length; i++) {
    const bit = bitForCharCode(lowerText.charCodeAt(i));
    if (bit < 0) continue;
    if (bit < 32) {
      lo |= 1 << bit;
    } else {
      hi |= 1 << (bit - 32);
    }
  }
  // Coerce to unsigned 32-bit.
  return { lo: lo >>> 0, hi: hi >>> 0 };
}

/**
 * Prefilter test: does `entry` contain all character classes present in
 * `query`?  Both are given as {lo, hi} word pairs. Pure integer ops, no BigInt.
 */
export function maskContains(
  entryLo: number,
  entryHi: number,
  queryLo: number,
  queryHi: number
): boolean {
  return (
    ((entryLo & queryLo) >>> 0) === queryLo &&
    ((entryHi & queryHi) >>> 0) === queryHi
  );
}

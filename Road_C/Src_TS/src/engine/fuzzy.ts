/*
 * fuzzy.ts — fzf-style subsequence scorer, adapted from Cling's Phase 2
 * (see ../../../open-source-analysis.md §3.4).
 *
 * Given a lowercase `pattern` and a lowercase `text`, we find whether pattern is
 * a subsequence of text and, if so, produce a score that rewards:
 *   - matches at word boundaries (after '/', '_', '-', '.', space, camelCase)
 *   - runs of consecutive matched characters
 *   - matches near the start of the (base)name
 * and penalises gaps between matched characters.
 *
 * On top of the raw fzf score we add a strong *substring / exact / prefix* tier
 * (see `substringBoost`). This is what keeps a clean hit like basename
 * "temp_test" for query "temp_test" far above scattered subsequence noise like
 * "testing_tools" or "vscode_pytest" — the requirement in
 * SEARCH_TEST_BASELINE.md (真目录排第 1).
 *
 * Scoring constants roughly follow the analysis table:
 *   char match +16, consecutive +4, first-char x2,
 *   word-boundary bonus +7..+9, gap start -3, gap continue -1.
 */

const SCORE_MATCH = 16;
const SCORE_CONSECUTIVE = 4;
const SCORE_BOUNDARY = 8;
const GAP_START = -3;
const GAP_EXTEND = -1;

// Substring / exact / prefix tier. These dwarf the per-char fzf score so any
// contiguous-substring hit ranks above every scattered-subsequence hit.
export const BOOST_EXACT = 4000; // text === pattern
export const BOOST_PREFIX = 2500; // text starts with pattern
export const BOOST_SUFFIX = 1500; // text ends with pattern (e.g. …/temp_test)
export const BOOST_SUBSTRING = 1000; // pattern occurs contiguously anywhere

function isBoundaryPrev(code: number): boolean {
  // A char preceded by one of these is treated as a word boundary.
  return (
    code === 47 || // '/'
    code === 95 || // '_'
    code === 45 || // '-'
    code === 46 || // '.'
    code === 32 // space
  );
}

/**
 * Contiguous-match boost for `pattern` inside `text` (both lowercase).
 * Returns 0 when `pattern` is not a contiguous substring of `text`.
 * Exact > prefix > suffix > interior; a boundary-aligned interior match
 * (pattern preceded by a separator) gets a little extra so "…/temp_test/…"
 * beats "…xtemp_testx…".
 */
export function substringBoost(pattern: string, text: string): number {
  if (pattern.length === 0) return 0;
  if (text === pattern) return BOOST_EXACT;
  const idx = text.indexOf(pattern);
  if (idx < 0) return 0;
  if (idx === 0) return BOOST_PREFIX;
  if (idx + pattern.length === text.length) return BOOST_SUFFIX;
  let boost = BOOST_SUBSTRING;
  if (isBoundaryPrev(text.charCodeAt(idx - 1))) boost += 200;
  return boost;
}

/**
 * Returns a score (higher = better) if `pattern` is a subsequence of `text`,
 * or null if it isn't. `baseStart` is the index in `text` where the basename
 * begins (matches inside the basename score higher). Empty pattern -> score 0.
 */
export function fuzzyScore(
  pattern: string,
  text: string,
  baseStart: number
): number | null {
  const plen = pattern.length;
  if (plen === 0) return 0;
  const tlen = text.length;
  if (plen > tlen) return null;

  let score = 0;
  let ti = 0;
  let prevMatchIndex = -2;

  for (let pi = 0; pi < plen; pi++) {
    const pc = pattern.charCodeAt(pi);
    // Advance through text until we find pc.
    while (ti < tlen && text.charCodeAt(ti) !== pc) ti++;
    if (ti >= tlen) return null; // pattern char not found -> not a subsequence

    score += SCORE_MATCH;

    // Consecutive-match bonus.
    if (ti === prevMatchIndex + 1) {
      score += SCORE_CONSECUTIVE;
    } else if (prevMatchIndex >= 0) {
      // Gap penalty proportional to gap size.
      const gap = ti - prevMatchIndex - 1;
      score += GAP_START + GAP_EXTEND * Math.min(gap, 8);
    }

    // Word-boundary bonus.
    if (ti === 0 || ti === baseStart || isBoundaryPrev(text.charCodeAt(ti - 1))) {
      score += SCORE_BOUNDARY;
    }

    // Basename bonus: matches at/after the basename start are worth a little more.
    if (ti >= baseStart) score += 2;

    prevMatchIndex = ti;
    ti++;
  }

  // First-char-at-start amplification.
  if (text.charCodeAt(0) === pattern.charCodeAt(0)) {
    score = Math.floor(score * 1.2);
  }

  return score;
}

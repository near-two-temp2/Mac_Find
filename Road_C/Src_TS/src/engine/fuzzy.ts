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
 * Scoring constants roughly follow the analysis table:
 *   char match +16, consecutive +4, first-char x2,
 *   word-boundary bonus +7..+9, gap start -3, gap continue -1.
 */

const SCORE_MATCH = 16;
const SCORE_CONSECUTIVE = 4;
const SCORE_BOUNDARY = 8;
const GAP_START = -3;
const GAP_EXTEND = -1;

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

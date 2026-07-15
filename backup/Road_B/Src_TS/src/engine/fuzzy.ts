// fuzzy.ts — fzf-style fuzzy scoring, Phase-2 of the search.
//
// Mirrors Cling's scorer (open-source-analysis.md §3.4): enumerate anchors of
// the first pattern byte, forward-greedy match, reverse-tighten to the most
// compact alignment, then score with boundary / consecutive bonuses and gap
// penalties. Operates on lowercase byte arrays so the whole hot path is
// integer comparisons.

// Scoring weights (kept in the same ballpark as Cling's table).
const SCORE_MATCH = 16;
const SCORE_CONSECUTIVE = 4;
const BONUS_FIRST_CHAR = 2; // multiplier on the first matched char's base
const BONUS_BOUNDARY = 8;
const GAP_START = -3;
const GAP_EXTEND = -1;
const MAX_ANCHORS = 32;

export interface FuzzyResult {
  score: number;
  start: number; // match start within text
  end: number; // match end (exclusive) within text
}

// Find the next occurrence of `needle` in text[from..count).
function findByte(text: Uint8Array, from: number, count: number, needle: number): number {
  for (let i = from; i < count; i++) {
    if (text[i] === needle) return i;
  }
  return -1;
}

// Score `pattern` against `text` (both lowercase bytes). `boundaries` is the
// basename word-boundary bitmap; `boundariesOffset` maps a text index to a bit
// position (text index - basename start). Returns null on no match.
export function fuzzyScore(
  pattern: Uint8Array,
  patLen: number,
  text: Uint8Array,
  textLen: number,
  boundaries: bigint,
  boundariesOffset: number
): FuzzyResult | null {
  if (patLen === 0) return { score: 0, start: 0, end: 0 };
  if (patLen > textLen) return null;

  const first = pattern[0];
  let best: FuzzyResult | null = null;

  // Enumerate up to MAX_ANCHORS starting positions where pattern[0] occurs.
  let anchorsSeen = 0;
  let a = findByte(text, 0, textLen, first);
  while (a >= 0 && anchorsSeen < MAX_ANCHORS) {
    anchorsSeen++;

    // Forward greedy: from anchor `a`, match every pattern byte in order.
    let ti = a;
    let matchedEnd = -1;
    for (let pi = 0; pi < patLen; pi++) {
      const idx = findByte(text, ti, textLen, pattern[pi]);
      if (idx < 0) {
        matchedEnd = -1;
        break;
      }
      ti = idx + 1;
      matchedEnd = idx;
    }

    if (matchedEnd >= 0) {
      // Reverse tighten: walk pattern backwards from matchedEnd to find the
      // most compact window (largest possible start), then score it.
      const window = tightenAndScore(
        pattern,
        patLen,
        text,
        a,
        matchedEnd,
        boundaries,
        boundariesOffset
      );
      if (best === null || window.score > best.score) best = window;
    }

    a = findByte(text, a + 1, textLen, first);
  }

  return best;
}

function bitSet(mask: bigint, pos: number): boolean {
  if (pos < 0 || pos >= 64) return false;
  return (mask & (1n << BigInt(pos))) !== 0n;
}

// Reverse-match pattern into text[.. matchedEnd], collecting matched indices,
// then compute a score with boundary and consecutive bonuses.
function tightenAndScore(
  pattern: Uint8Array,
  patLen: number,
  text: Uint8Array,
  anchor: number,
  matchedEnd: number,
  boundaries: bigint,
  boundariesOffset: number
): FuzzyResult {
  // Collect matched text indices by scanning backwards from matchedEnd.
  const idxs = new Array<number>(patLen);
  let ti = matchedEnd;
  for (let pi = patLen - 1; pi >= 0; pi--) {
    while (ti >= anchor && text[ti] !== pattern[pi]) ti--;
    idxs[pi] = ti;
    ti--;
  }
  // idxs[0] is the tightened start (>= anchor). idxs is strictly increasing.

  let score = 0;
  let prevIdx = -2;
  for (let pi = 0; pi < patLen; pi++) {
    const t = idxs[pi];
    let base = SCORE_MATCH;

    // Word-boundary bonus.
    if (bitSet(boundaries, t - boundariesOffset)) base += BONUS_BOUNDARY;

    if (pi === 0) {
      base *= BONUS_FIRST_CHAR;
    } else if (t === prevIdx + 1) {
      base += SCORE_CONSECUTIVE; // consecutive match
    } else {
      // Gap between this match and the previous one.
      const gap = t - prevIdx - 1;
      base += GAP_START + GAP_EXTEND * (gap - 1);
    }
    score += base;
    prevIdx = t;
  }

  return { score, start: idxs[0], end: idxs[patLen - 1] + 1 };
}

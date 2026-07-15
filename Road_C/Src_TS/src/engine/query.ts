/*
 * query.ts — two-phase search over a loaded IndexView.
 *
 *   Phase 1: O(n) parallel-friendly prefilter
 *            - bitmask test (query char classes must be a subset of entry's)
 *            - file/dir type filter
 *   Phase 2: fzf scoring of the survivors, then top-K by score.
 *
 * Runs over an arbitrary [start, end) slice so it can be sharded across
 * worker_threads (see searchWorker.ts) — each worker scores its own slice and
 * the coordinator merges the per-shard top-K lists.
 */

import { maskWords, maskContains } from './bitmask';
import { fuzzyScore } from './fuzzy';
import type { IndexView } from './binaryIndex';

export interface QueryOptions {
  dirsOnly?: boolean;
  filesOnly?: boolean;
  limit?: number; // max results returned
}

export interface ScoredHit {
  index: number; // entry index into the IndexView
  score: number;
}

export interface QueryResult {
  path: string;
  isDir: boolean;
  score: number;
}

const decoder = new TextDecoder();

/** Score one slice [start, end) of the index, returning up to `limit` hits. */
export function scoreSlice(
  view: IndexView,
  patternLower: string,
  start: number,
  end: number,
  opts: QueryOptions
): ScoredHit[] {
  const { lo: qLo, hi: qHi } = maskWords(patternLower);
  const limit = opts.limit ?? 200;
  const hits: ScoredHit[] = [];

  const { maskLo, maskHi, byteOffset, byteLen, baseStart, isDir, lowerBlob } =
    view;

  for (let i = start; i < end; i++) {
    // Type filter.
    if (opts.dirsOnly && isDir[i] === 0) continue;
    if (opts.filesOnly && isDir[i] !== 0) continue;

    // Phase 1: bitmask prefilter (skips the vast majority of entries).
    if (!maskContains(maskLo[i], maskHi[i], qLo, qHi)) continue;

    // Phase 2: decode this entry's lowercase path and score it.
    const off = byteOffset[i];
    const len = byteLen[i];
    const text = decoder.decode(lowerBlob.subarray(off, off + len));
    const s = fuzzyScore(patternLower, text, baseStart[i]);
    if (s === null) continue;

    hits.push({ index: i, score: s });
  }

  // Partial sort: keep top `limit` by score (desc), tie-break by shorter path.
  hits.sort((a, b) => {
    if (b.score !== a.score) return b.score - a.score;
    return byteLen[a.index] - byteLen[b.index];
  });
  if (hits.length > limit) hits.length = limit;
  return hits;
}

/** Materialize scored hits into displayable results using original-case paths. */
export function materialize(view: IndexView, hits: ScoredHit[]): QueryResult[] {
  const out: QueryResult[] = new Array(hits.length);
  for (let k = 0; k < hits.length; k++) {
    const i = hits[k].index;
    const off = view.byteOffset[i];
    const len = view.byteLen[i];
    const orig = decoder.decode(view.origBlob.subarray(off, off + len));
    out[k] = {
      path: orig,
      isDir: view.isDir[i] !== 0,
      score: hits[k].score,
    };
  }
  return out;
}

/** Single-threaded whole-index query. Used by the CLI and as a worker fallback. */
export function queryIndex(
  view: IndexView,
  pattern: string,
  opts: QueryOptions
): QueryResult[] {
  const patternLower = pattern.toLowerCase();
  const hits = scoreSlice(view, patternLower, 0, view.entryCount, opts);
  return materialize(view, hits);
}

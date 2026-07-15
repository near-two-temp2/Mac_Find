// searcher.ts — two-phase search over a loaded IndexReader.
//
// Phase 1 (O(n) tight scan): a single UInt64 bitmask AND rejects entries whose
//   basename can't contain every query letter, plus an optional exact
//   extension-id equality check and dir/file/depth filters. This is the cheap
//   pass that Cling parallelizes across cores (§3.4 Phase 1).
// Phase 2 (fzf): only survivors are scored with fuzzy.ts, then the top-k by
//   score are returned. The query is matched against the basename first (the
//   common case) and falls back to the whole path.

import { IndexReader } from "./indexReader";
import { computeMask } from "./indexFormat";
import { fuzzyScore } from "./fuzzy";

export interface SearchOptions {
  limit?: number; // max results (default 200)
  filesOnly?: boolean;
  dirsOnly?: boolean;
  maxDepth?: number; // segCounts cap
  matchWholePath?: boolean; // score against full path, not just basename
}

export interface SearchHit {
  path: string; // lowercase indexed path
  score: number;
  isDir: boolean;
  start: number;
  end: number;
}

export class Searcher {
  constructor(private idx: IndexReader) {}

  search(queryRaw: string, opts: SearchOptions = {}): SearchHit[] {
    const idx = this.idx;
    const limit = opts.limit ?? 200;
    const query = queryRaw.toLowerCase();
    const enc = new TextEncoder();
    const patBytes = enc.encode(query);
    const patLen = patBytes.length;

    // Empty query: return the first `limit` entries (a cheap "browse" mode).
    if (patLen === 0) {
      const hits: SearchHit[] = [];
      for (let i = 0; i < idx.entryCount && hits.length < limit; i++) {
        if (!this.passesTypeDepth(i, opts)) continue;
        hits.push({ path: idx.pathAt(i), score: 0, isDir: idx.isDirs[i] === 1, start: 0, end: 0 });
      }
      return hits;
    }

    // Phase-1 query mask (over the pattern's own bytes).
    const queryMask = computeMask(patBytes, 0, patLen);

    // Optional exact-extension filter: if the query looks like "*.ext" or ends
    // with ".ext", require that extension id. Kept conservative: only when the
    // query itself contains a dot in a trailing position.
    const wantExtId = this.extFilterId(query);

    const survivors: number[] = [];
    const n = idx.entryCount;
    for (let i = 0; i < n; i++) {
      // Bitmask prefilter against basename mask (query letters must all fit).
      if ((idx.bnMasks[i] & queryMask) !== queryMask) {
        // Fall back to full-path mask before rejecting (query may target dir).
        if ((idx.masks[i] & queryMask) !== queryMask) continue;
      }
      if (wantExtId > 0 && idx.extIds[i] !== wantExtId) continue;
      if (!this.passesTypeDepth(i, opts)) continue;
      survivors.push(i);
    }

    // Phase-2: fzf score each survivor.
    const scored: SearchHit[] = [];
    const textBuf = idx.allBytes;
    for (const i of survivors) {
      const off = idx.byteOffsets[i];
      const len = idx.byteLengths[i];
      const bnStart = idx.bnStarts[i];

      // Prefer matching within the basename (offset-relative), fall back to path.
      let res = fuzzyScore(
        patBytes,
        patLen,
        textBuf.subarray(off + bnStart, off + len),
        len - bnStart,
        idx.bnBoundaries[i],
        0
      );
      let base = off + bnStart;

      if (res === null || opts.matchWholePath) {
        const pathRes = fuzzyScore(
          patBytes,
          patLen,
          textBuf.subarray(off, off + len),
          len,
          idx.bnBoundaries[i],
          bnStart
        );
        if (pathRes !== null && (res === null || pathRes.score > res.score)) {
          res = pathRes;
          base = off;
        }
      }

      if (res === null) continue;
      scored.push({
        path: idx.pathAt(i),
        score: res.score,
        isDir: idx.isDirs[i] === 1,
        start: base - off + res.start,
        end: base - off + res.end,
      });
    }

    scored.sort((a, b) => b.score - a.score || a.path.length - b.path.length);
    return scored.slice(0, limit);
  }

  private passesTypeDepth(i: number, opts: SearchOptions): boolean {
    const isDir = this.idx.isDirs[i] === 1;
    if (opts.filesOnly && isDir) return false;
    if (opts.dirsOnly && !isDir) return false;
    if (opts.maxDepth !== undefined && this.idx.segCounts[i] > opts.maxDepth) return false;
    return true;
  }

  // If the query trailing-ends in ".ext", map to an extension id for a hard
  // Phase-1 filter; otherwise 0 (no extension filter).
  private extFilterId(query: string): number {
    const dot = query.lastIndexOf(".");
    if (dot <= 0 || dot === query.length - 1) return 0;
    // Only treat as an extension filter if what follows the dot is short and
    // alphanumeric (avoids treating "foo.bar/baz" as an ext query).
    const ext = query.slice(dot + 1);
    if (ext.length > 10 || /[^a-z0-9]/.test(ext)) return 0;
    return this.idx.extTable.idFor(ext);
  }
}

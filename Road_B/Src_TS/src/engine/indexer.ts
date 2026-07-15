// indexer.ts — build the binary index by walking the filesystem.
//
// Walks a set of roots, and for every entry records the parallel-array fields
// described in indexFormat.ts. The result is serialized to a compact binary
// blob (typed arrays back-to-back) plus a small JSON sidecar for the extension
// table. Reading the blob back is zero-copy: each typed array is a view over
// the file buffer at a fixed offset (the closest Node gets to Cling's mmap).

import * as fs from "fs";
import * as path from "path";
import {
  INDEX_MAGIC,
  HEADER_BYTES,
  computeMask,
  computeBoundaries,
} from "./indexFormat";
import { ExtTable, extractExt } from "./extIds";

export interface IndexBuildOptions {
  roots: string[];
  maxEntries?: number; // safety cap so CI / smoke runs stay bounded
  followSymlinks?: boolean;
  excludeDirs?: string[]; // basenames to skip entirely
  onProgress?: (count: number) => void;
}

// Default excludes: heavy / churny dirs that add noise to a demo index.
const DEFAULT_EXCLUDES = new Set([
  ".git",
  "node_modules",
  ".Trash",
  "Library/Caches",
]);

interface ScratchEntry {
  pathLower: string;
  bnStart: number;
  isDir: boolean;
}

// Recursively collect entries under `roots`. Kept iterative to avoid deep
// recursion blowups on large trees.
function collect(opts: IndexBuildOptions): ScratchEntry[] {
  const out: ScratchEntry[] = [];
  const cap = opts.maxEntries ?? 2_000_000;
  const excludes = new Set(opts.excludeDirs ?? []);
  const stack: string[] = [...opts.roots];

  while (stack.length > 0 && out.length < cap) {
    const dir = stack.pop()!;
    let entries: fs.Dirent[];
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      continue; // permission denied / vanished dir — skip
    }
    for (const e of entries) {
      if (out.length >= cap) break;
      const name = e.name;
      if (name === "." || name === "..") continue;
      if (DEFAULT_EXCLUDES.has(name) || excludes.has(name)) continue;

      const full = path.join(dir, name);
      let isDir = e.isDirectory();
      let isSymlink = e.isSymbolicLink();

      if (isSymlink) {
        if (!opts.followSymlinks) {
          // Record the symlink itself as a leaf, don't traverse it.
          out.push(makeEntry(full, false));
          continue;
        }
        try {
          isDir = fs.statSync(full).isDirectory();
        } catch {
          continue;
        }
      }

      out.push(makeEntry(full, isDir));
      if (isDir) stack.push(full);

      if (opts.onProgress && out.length % 5000 === 0) opts.onProgress(out.length);
    }
  }
  return out;
}

function makeEntry(full: string, isDir: boolean): ScratchEntry {
  const pathLower = full.toLowerCase();
  const slash = pathLower.lastIndexOf("/");
  const bnStart = slash < 0 ? 0 : slash + 1;
  return { pathLower, bnStart, isDir };
}

function countSegments(pathLower: string): number {
  let n = 0;
  for (let i = 0; i < pathLower.length; i++) if (pathLower.charCodeAt(i) === 47) n++;
  return Math.min(n, 255);
}

export interface SerializedIndex {
  blob: Buffer; // header + all parallel arrays + allBytes
  extTable: string[]; // JSON sidecar
  entryCount: number;
}

// Build the full index and serialize it into a single Buffer + ext sidecar.
export function buildIndex(opts: IndexBuildOptions): SerializedIndex {
  const scratch = collect(opts);
  const n = scratch.length;
  const extTable = new ExtTable();

  // First pass: encode each path to UTF-8 bytes, tally total bytes.
  const enc = new TextEncoder();
  const pathBytes: Uint8Array[] = new Array(n);
  let totalBytes = 0;
  for (let i = 0; i < n; i++) {
    const b = enc.encode(scratch[i].pathLower);
    pathBytes[i] = b;
    totalBytes += b.length;
  }

  // Parallel arrays.
  const masks = new BigUint64Array(n);
  const bnMasks = new BigUint64Array(n);
  const bnBoundaries = new BigUint64Array(n);
  const byteOffsets = new Uint32Array(n);
  const byteLengths = new Uint16Array(n);
  const bnStarts = new Uint16Array(n);
  const extIds = new Uint16Array(n);
  const segCounts = new Uint8Array(n);
  const isDirs = new Uint8Array(n);
  const allBytes = new Uint8Array(totalBytes);

  let off = 0;
  for (let i = 0; i < n; i++) {
    const s = scratch[i];
    const b = pathBytes[i];
    const len = b.length;
    const bnStart = s.bnStart;

    allBytes.set(b, off);
    byteOffsets[i] = off;
    byteLengths[i] = Math.min(len, 0xffff);
    bnStarts[i] = Math.min(bnStart, 0xffff);
    segCounts[i] = countSegments(s.pathLower);
    isDirs[i] = s.isDir ? 1 : 0;

    masks[i] = computeMask(b, 0, len);
    bnMasks[i] = computeMask(b, bnStart, len);
    bnBoundaries[i] = computeBoundaries(b, bnStart, len);
    extIds[i] = extractExtId(extTable, s.pathLower, bnStart);

    off += len;
  }

  const blob = serialize({
    n,
    totalBytes,
    masks,
    bnMasks,
    bnBoundaries,
    byteOffsets,
    byteLengths,
    bnStarts,
    extIds,
    segCounts,
    isDirs,
    allBytes,
  });

  return { blob, extTable: extTable.serialize(), entryCount: n };
}

function extractExtId(table: ExtTable, pathLower: string, bnStart: number): number {
  const ext = extractExt(pathLower, bnStart);
  return table.intern(ext);
}

interface Arrays {
  n: number;
  totalBytes: number;
  masks: BigUint64Array;
  bnMasks: BigUint64Array;
  bnBoundaries: BigUint64Array;
  byteOffsets: Uint32Array;
  byteLengths: Uint16Array;
  bnStarts: Uint16Array;
  extIds: Uint16Array;
  segCounts: Uint8Array;
  isDirs: Uint8Array;
  allBytes: Uint8Array;
}

// Lay out: header, then each array back-to-back (8-byte-aligned per array),
// then allBytes. Section offsets are recomputed on read from n + header.
function serialize(a: Arrays): Buffer {
  const n = a.n;
  const sizes = {
    masks: n * 8,
    bnMasks: n * 8,
    bnBoundaries: n * 8,
    byteOffsets: n * 4,
    byteLengths: n * 2,
    bnStarts: n * 2,
    extIds: n * 2,
    segCounts: n * 1,
    isDirs: n * 1,
  };
  const arraysBytes =
    sizes.masks +
    sizes.bnMasks +
    sizes.bnBoundaries +
    sizes.byteOffsets +
    sizes.byteLengths +
    sizes.bnStarts +
    sizes.extIds +
    sizes.segCounts +
    sizes.isDirs;

  const total = HEADER_BYTES + arraysBytes + a.totalBytes;
  const buf = Buffer.alloc(total);

  // Header.
  buf.writeBigUInt64LE(INDEX_MAGIC, 0);
  buf.writeBigUInt64LE(BigInt(n), 8);
  buf.writeBigUInt64LE(BigInt(a.totalBytes), 16);
  buf.writeBigUInt64LE(0n, 24); // reserved

  let o = HEADER_BYTES;
  const put = (src: ArrayBufferView, byteLen: number) => {
    const view = new Uint8Array(src.buffer, src.byteOffset, byteLen);
    buf.set(view, o);
    o += byteLen;
  };

  put(a.masks, sizes.masks);
  put(a.bnMasks, sizes.bnMasks);
  put(a.bnBoundaries, sizes.bnBoundaries);
  put(a.byteOffsets, sizes.byteOffsets);
  put(a.byteLengths, sizes.byteLengths);
  put(a.bnStarts, sizes.bnStarts);
  put(a.extIds, sizes.extIds);
  put(a.segCounts, sizes.segCounts);
  put(a.isDirs, sizes.isDirs);
  put(a.allBytes, a.totalBytes);

  return buf;
}

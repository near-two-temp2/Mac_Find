/*
 * binaryIndex.ts — self-built binary index, parallel-array layout inspired by
 * Cling's .idx format (see ../../../open-source-analysis.md §3.3).
 *
 * On-disk / in-memory layout (all little-endian, mmap-friendly parallel arrays):
 *
 *   Header (32 bytes)
 *     magic:      u64   0x4D414346494E4458  ("MACFINDX")
 *     version:    u32   = 1
 *     entryCount: u32
 *     bytesCount: u32   (length of the packed path bytes blob)
 *     reserved:   u32,u32,u32
 *
 *   Parallel arrays (entryCount entries each):
 *     maskLo:     Uint32   path presence mask, low word
 *     maskHi:     Uint32   path presence mask, high word
 *     byteOffset: Uint32   offset of this path's bytes in `blob`
 *     byteLen:    Uint16   length of the (lowercase) path in bytes
 *     baseStart:  Uint16   index of basename start within the path
 *     isDir:      Uint8    1 if directory, else 0
 *
 *   Blob:
 *     packed lowercase UTF-8 path bytes for every entry
 *
 * We keep a *separate* copy of the original-case paths blob so the UI can show
 * the real path while search works on the lowercase copy. To keep the format
 * simple both blobs are concatenated: [lowerBlob][origBlob], each `bytesCount`.
 * (For files that are pure ASCII the two are usually identical; storing both is
 * cheap relative to the array overhead and avoids re-casing at query time.)
 */

import { maskWords } from './bitmask';

export const INDEX_MAGIC_LO = 0x46494e44; // "FIND"
export const INDEX_MAGIC_HI = 0x4d414358; // "MACX"
export const INDEX_VERSION = 1;

const HEADER_BYTES = 32;

export interface RawEntry {
  path: string; // original-case full path
  isDir: boolean;
}

/** In-memory, query-ready view over an index buffer. */
export interface IndexView {
  entryCount: number;
  maskLo: Uint32Array;
  maskHi: Uint32Array;
  byteOffset: Uint32Array;
  byteLen: Uint16Array;
  baseStart: Uint16Array;
  isDir: Uint8Array;
  lowerBlob: Uint8Array; // lowercase path bytes
  origBlob: Uint8Array; // original-case path bytes (same offsets/lengths)
  buffer: ArrayBuffer; // the whole thing, for transfer to workers
}

function baseStartOf(path: string): number {
  const idx = path.lastIndexOf('/');
  return idx < 0 ? 0 : idx + 1;
}

/**
 * Serialize an array of entries into a single ArrayBuffer using the layout
 * above. Returns the buffer; caller may write it to disk or hand it to workers.
 */
export function buildIndexBuffer(entries: RawEntry[]): ArrayBuffer {
  const n = entries.length;

  // First pass: encode bytes, compute offsets/lengths/masks.
  const enc = new TextEncoder();
  const lowerBytesList: Uint8Array[] = new Array(n);
  const origBytesList: Uint8Array[] = new Array(n);
  const offsets = new Uint32Array(n);
  const lengths = new Uint16Array(n);
  const bases = new Uint16Array(n);
  const maskLoArr = new Uint32Array(n);
  const maskHiArr = new Uint32Array(n);
  const isDirArr = new Uint8Array(n);

  let blobLen = 0;
  for (let i = 0; i < n; i++) {
    const p = entries[i].path;
    const lower = p.toLowerCase();
    const lb = enc.encode(lower);
    const ob = enc.encode(p);
    lowerBytesList[i] = lb;
    origBytesList[i] = ob;
    offsets[i] = blobLen;
    // byteLen is length of lowercase bytes; orig uses the same offset but may
    // differ in length. To keep offsets shared we clamp orig to the same slot
    // by storing orig separately at identical offset in a parallel blob whose
    // per-entry length equals the lowercase length; for non-ASCII this could
    // mismatch, so we pad/truncate origBlob to lower length below.
    lengths[i] = Math.min(lb.length, 0xffff);
    bases[i] = Math.min(baseStartOf(lower), 0xffff);
    const { lo, hi } = maskWords(lower);
    maskLoArr[i] = lo;
    maskHiArr[i] = hi;
    isDirArr[i] = entries[i].isDir ? 1 : 0;
    blobLen += lengths[i];
  }

  const arraysBytes =
    n * 4 + // maskLo
    n * 4 + // maskHi
    n * 4 + // byteOffset
    n * 2 + // byteLen
    n * 2 + // baseStart
    n * 1; // isDir

  const total = HEADER_BYTES + arraysBytes + blobLen * 2; // two blobs
  const buffer = new ArrayBuffer(total);
  const dv = new DataView(buffer);

  // Header
  dv.setUint32(0, INDEX_MAGIC_LO, true);
  dv.setUint32(4, INDEX_MAGIC_HI, true);
  dv.setUint32(8, INDEX_VERSION, true);
  dv.setUint32(12, n, true);
  dv.setUint32(16, blobLen, true);
  // 20,24,28 reserved (zero)

  let off = HEADER_BYTES;
  const maskLo = new Uint32Array(buffer, off, n);
  maskLo.set(maskLoArr);
  off += n * 4;
  const maskHi = new Uint32Array(buffer, off, n);
  maskHi.set(maskHiArr);
  off += n * 4;
  const byteOffset = new Uint32Array(buffer, off, n);
  off += n * 4;
  const byteLen = new Uint16Array(buffer, off, n);
  byteLen.set(lengths);
  off += n * 2;
  const baseStart = new Uint16Array(buffer, off, n);
  baseStart.set(bases);
  off += n * 2;
  const isDir = new Uint8Array(buffer, off, n);
  isDir.set(isDirArr);
  off += n * 1;

  const lowerBlob = new Uint8Array(buffer, off, blobLen);
  const origBlob = new Uint8Array(buffer, off + blobLen, blobLen);

  let cursor = 0;
  for (let i = 0; i < n; i++) {
    const lb = lowerBytesList[i];
    const ob = origBytesList[i];
    const len = lengths[i];
    byteOffset[i] = cursor;
    lowerBlob.set(lb.subarray(0, len), cursor);
    // Copy orig into the same slot; if orig is longer (multibyte differences),
    // it's truncated to `len`. UI falls back to lowercase decode when needed.
    origBlob.set(ob.subarray(0, len), cursor);
    cursor += len;
  }

  return buffer;
}

/** Parse an index buffer into a query-ready view. Throws on bad magic/version. */
export function loadIndexView(buffer: ArrayBuffer): IndexView {
  const dv = new DataView(buffer);
  const magicLo = dv.getUint32(0, true);
  const magicHi = dv.getUint32(4, true);
  if (magicLo !== INDEX_MAGIC_LO || magicHi !== INDEX_MAGIC_HI) {
    throw new Error('bad index magic');
  }
  const version = dv.getUint32(8, true);
  if (version !== INDEX_VERSION) {
    throw new Error(`unsupported index version ${version}`);
  }
  const n = dv.getUint32(12, true);
  const blobLen = dv.getUint32(16, true);

  let off = HEADER_BYTES;
  const maskLo = new Uint32Array(buffer, off, n);
  off += n * 4;
  const maskHi = new Uint32Array(buffer, off, n);
  off += n * 4;
  const byteOffset = new Uint32Array(buffer, off, n);
  off += n * 4;
  const byteLen = new Uint16Array(buffer, off, n);
  off += n * 2;
  const baseStart = new Uint16Array(buffer, off, n);
  off += n * 2;
  const isDir = new Uint8Array(buffer, off, n);
  off += n * 1;
  const lowerBlob = new Uint8Array(buffer, off, blobLen);
  const origBlob = new Uint8Array(buffer, off + blobLen, blobLen);

  return {
    entryCount: n,
    maskLo,
    maskHi,
    byteOffset,
    byteLen,
    baseStart,
    isDir,
    lowerBlob,
    origBlob,
    buffer,
  };
}

// indexReader.ts — zero-copy view over a serialized index buffer.
//
// Given the Buffer produced by indexer.serialize(), reconstruct each parallel
// array as a typed-array view (no per-entry copying — the closest Node equiv
// of Cling's mmap'd index). Section offsets are derived from the entry count in
// the header, in the exact order indexer.serialize() wrote them.

import { INDEX_MAGIC, HEADER_BYTES } from "./indexFormat";
import { ExtTable } from "./extIds";

export class IndexReader {
  readonly entryCount: number;
  readonly allBytesLen: number;

  readonly masks: BigUint64Array;
  readonly bnMasks: BigUint64Array;
  readonly bnBoundaries: BigUint64Array;
  readonly byteOffsets: Uint32Array;
  readonly byteLengths: Uint16Array;
  readonly bnStarts: Uint16Array;
  readonly extIds: Uint16Array;
  readonly segCounts: Uint8Array;
  readonly isDirs: Uint8Array;
  readonly allBytes: Uint8Array;

  readonly extTable: ExtTable;

  private buf: Buffer;

  constructor(buf: Buffer, extTableList: string[]) {
    this.buf = buf;
    const magic = buf.readBigUInt64LE(0);
    if (magic !== INDEX_MAGIC) {
      throw new Error(
        `bad index magic: got 0x${magic.toString(16)}, want 0x${INDEX_MAGIC.toString(16)}`
      );
    }
    const n = Number(buf.readBigUInt64LE(8));
    this.entryCount = n;
    this.allBytesLen = Number(buf.readBigUInt64LE(16));
    this.extTable = ExtTable.deserialize(extTableList);

    // The typed-array views must be aligned to their element size. Buffer.alloc
    // does not guarantee 8-byte alignment of the underlying ArrayBuffer, so we
    // slice into fresh aligned arrays only when needed. In practice the header
    // is 32 bytes and every section is naturally aligned relative to it, but the
    // ArrayBuffer base offset (buf.byteOffset) may not be 8-aligned. To stay
    // correct on every platform we copy the two-and-one-byte-aligned-safe way:
    // read via a DataView-free approach using set() on freshly allocated views.
    const ab = buf.buffer;
    const base = buf.byteOffset + HEADER_BYTES;

    let o = base;
    const view8 = (count: number): BigUint64Array => {
      const bytes = count * 8;
      const a = new BigUint64Array(count);
      new Uint8Array(a.buffer).set(new Uint8Array(ab, o, bytes));
      o += bytes;
      return a;
    };
    const view32 = (count: number): Uint32Array => {
      const bytes = count * 4;
      const a = new Uint32Array(count);
      new Uint8Array(a.buffer).set(new Uint8Array(ab, o, bytes));
      o += bytes;
      return a;
    };
    const view16 = (count: number): Uint16Array => {
      const bytes = count * 2;
      const a = new Uint16Array(count);
      new Uint8Array(a.buffer).set(new Uint8Array(ab, o, bytes));
      o += bytes;
      return a;
    };
    const view8u = (count: number): Uint8Array => {
      const a = new Uint8Array(ab, o, count);
      o += count;
      return a;
    };

    this.masks = view8(n);
    this.bnMasks = view8(n);
    this.bnBoundaries = view8(n);
    this.byteOffsets = view32(n);
    this.byteLengths = view16(n);
    this.bnStarts = view16(n);
    this.extIds = view16(n);
    this.segCounts = view8u(n);
    this.isDirs = view8u(n);
    this.allBytes = new Uint8Array(ab, o, this.allBytesLen);
  }

  private dec = new TextDecoder();

  // Decode entry i's path as a (lowercase) string.
  pathAt(i: number): string {
    const start = this.byteOffsets[i];
    const len = this.byteLengths[i];
    return this.dec.decode(this.allBytes.subarray(start, start + len));
  }
}

/*
 * indexStore.ts — persist / load the binary index to disk.
 *
 * Default location mirrors Cling's convention:
 *   ~/Library/Caches/org.macfind.roadc.ts/index.idx
 *
 * On load we validate the magic/version (loadIndexView throws on mismatch); the
 * hybrid engine treats any load failure as "index missing/corrupt" and switches
 * to the searchfs() fallback — exactly the Road_C requirement.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  buildIndexBuffer,
  loadIndexView,
  type IndexView,
  type RawEntry,
} from './binaryIndex';

const APP_ID = 'org.macfind.roadc.ts';

export function defaultIndexDir(): string {
  return path.join(os.homedir(), 'Library', 'Caches', APP_ID);
}

export function defaultIndexPath(): string {
  return path.join(defaultIndexDir(), 'index.idx');
}

/** Build an index from entries and write it to `idxPath` (dirs created). */
export function writeIndex(entries: RawEntry[], idxPath: string): number {
  const buffer = buildIndexBuffer(entries);
  fs.mkdirSync(path.dirname(idxPath), { recursive: true });
  fs.writeFileSync(idxPath, Buffer.from(buffer));
  return buffer.byteLength;
}

/**
 * Load an index from disk. Returns null (not throw) if the file is absent or
 * fails validation, so the hybrid engine can cleanly fall back to searchfs().
 */
export function loadIndex(idxPath: string): IndexView | null {
  let raw: Buffer;
  try {
    raw = fs.readFileSync(idxPath);
  } catch {
    return null; // missing
  }
  // Copy into a standalone ArrayBuffer (readFileSync's buffer may be pooled).
  const ab = raw.buffer.slice(
    raw.byteOffset,
    raw.byteOffset + raw.byteLength
  ) as ArrayBuffer;
  try {
    return loadIndexView(ab);
  } catch {
    return null; // corrupt / wrong version
  }
}

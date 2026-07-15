// store.ts — persist / load the index blob and its extension sidecar.
//
// The binary blob goes to <cacheDir>/index.mffbidx and the extension table to
// <cacheDir>/exttable.json, mirroring Cling's ~/Library/Caches/<bundle>/*.idx
// layout. cacheDir defaults to macOS's per-user Caches directory.

import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { IndexReader } from "./indexReader";
import { SerializedIndex } from "./indexer";

const BUNDLE = "com.machaifind.roadb.ts";

export function defaultCacheDir(): string {
  // macOS: ~/Library/Caches/<bundle>. Falls back to os.tmpdir elsewhere (CI on
  // non-mac dev boxes still works for smoke tests).
  const home = os.homedir();
  if (process.platform === "darwin") {
    return path.join(home, "Library", "Caches", BUNDLE);
  }
  return path.join(os.tmpdir(), BUNDLE);
}

function blobPath(dir: string): string {
  return path.join(dir, "index.mffbidx");
}
function extPath(dir: string): string {
  return path.join(dir, "exttable.json");
}

export interface IndexMeta {
  entryCount: number;
  builtAt: string; // ISO timestamp
  roots: string[];
}

export function saveIndex(
  serialized: SerializedIndex,
  roots: string[],
  dir: string = defaultCacheDir()
): IndexMeta {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(blobPath(dir), serialized.blob);
  const meta: IndexMeta = {
    entryCount: serialized.entryCount,
    builtAt: new Date().toISOString(),
    roots,
  };
  fs.writeFileSync(
    extPath(dir),
    JSON.stringify({ extTable: serialized.extTable, meta })
  );
  return meta;
}

export interface LoadedIndex {
  reader: IndexReader;
  meta: IndexMeta;
}

export function indexExists(dir: string = defaultCacheDir()): boolean {
  return fs.existsSync(blobPath(dir)) && fs.existsSync(extPath(dir));
}

export function loadIndex(dir: string = defaultCacheDir()): LoadedIndex {
  const blob = fs.readFileSync(blobPath(dir));
  const sidecar = JSON.parse(fs.readFileSync(extPath(dir), "utf8"));
  const extTable: string[] = sidecar.extTable ?? [""];
  const meta: IndexMeta = sidecar.meta ?? {
    entryCount: 0,
    builtAt: "unknown",
    roots: [],
  };
  const reader = new IndexReader(blob, extTable);
  return { reader, meta };
}

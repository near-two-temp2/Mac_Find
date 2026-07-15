// Mirror of the Rust engine's serde types (camelCase over the IPC boundary).

export interface Hit {
  path: string;
  name: string;
  isDir: boolean;
  score: number;
}

export type EngineKind = "index" | "searchfs";

export interface SearchResponse {
  engine: EngineKind;
  hits: Hit[];
  scanned: number;
  elapsedMs: number;
}

export interface EngineStatus {
  indexReady: boolean;
  indexEntries: number;
  searchfsAvailable: boolean;
  indexPath: string;
}

export interface SearchOptions {
  filesOnly?: boolean;
  dirsOnly?: boolean;
  caseSensitive?: boolean;
  skipPackages?: boolean;
  skipInvisibles?: boolean;
  matchStart?: boolean;
  matchEnd?: boolean;
  limit?: number;
}

/*
 * global.d.ts — ambient types for the renderer's window.macfind bridge.
 *
 * Kept as a global (no top-level import/export) so renderer.ts can stay a plain
 * browser script: tsc then emits classic <script>-compatible JS with no
 * CommonJS require()/exports wrappers.
 */

interface MacFindEngineStatus {
  hasIndex: boolean;
  entries: number;
  fallbackReady: boolean;
  fallbackError: string | null;
}

interface MacFindSearchHit {
  path: string;
  isDir: boolean;
  score: number;
}

interface MacFindSearchResult {
  mode: 'index' | 'searchfs' | 'none';
  results: MacFindSearchHit[];
  tookMs: number;
  note?: string;
}

interface MacFindApi {
  status(): Promise<MacFindEngineStatus>;
  search(
    pattern: string,
    opts?: { limit?: number; dirsOnly?: boolean; filesOnly?: boolean }
  ): Promise<MacFindSearchResult>;
  reindex(roots?: string[], max?: number): Promise<number>;
  reveal(filePath: string): Promise<boolean>;
  open(filePath: string): Promise<boolean | string>;
}

interface Window {
  macfind: MacFindApi;
}

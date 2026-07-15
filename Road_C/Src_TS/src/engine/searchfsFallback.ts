/*
 * searchfsFallback.ts — thin loader for the native searchfs() addon.
 *
 * The hybrid engine calls this when the binary index is missing or corrupt.
 * If the addon didn't build (non-macOS, or CI without the toolchain), we detect
 * that and report the fallback as unavailable rather than crashing.
 */

export interface SearchfsOpts {
  dirsOnly?: boolean;
  filesOnly?: boolean;
  exact?: boolean;
  limit?: number;
}

type Addon = {
  search(pattern: string, opts?: SearchfsOpts): string[];
};

let addon: Addon | null = null;
let loadError: string | null = null;

function tryLoad(): void {
  if (addon !== null || loadError !== null) return;
  try {
    // `bindings` resolves build/Release/searchfs_addon.node relative to package.
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const bindings = require('bindings');
    addon = bindings('searchfs_addon') as Addon;
  } catch (e) {
    loadError = e instanceof Error ? e.message : String(e);
    addon = null;
  }
}

/** True if the native searchfs() addon loaded successfully. */
export function searchfsAvailable(): boolean {
  tryLoad();
  return addon !== null;
}

export function searchfsLoadError(): string | null {
  tryLoad();
  return loadError;
}

/**
 * Live searchfs() search. Returns absolute paths. Throws if the addon is
 * unavailable (callers should check searchfsAvailable() first).
 */
export function searchfsSearch(pattern: string, opts?: SearchfsOpts): string[] {
  tryLoad();
  if (!addon) {
    throw new Error(`searchfs addon unavailable: ${loadError ?? 'not built'}`);
  }
  return addon.search(pattern, opts);
}

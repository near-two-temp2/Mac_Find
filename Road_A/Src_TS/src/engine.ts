/*
 * engine.ts — thin TS wrapper around the native searchfs addon.
 *
 * Loads build/Release/searchfs.node and exposes a typed search() function.
 * If the native addon is unavailable (e.g. running the CLI smoke test on a
 * non-macOS box, or before `npm run build`), it throws a clear error rather
 * than crashing the whole process, so callers can degrade gracefully.
 */

export interface SearchOptions {
  /** Match directories only. Mutually exclusive with filesOnly. */
  dirsOnly?: boolean;
  /** Match files only. Mutually exclusive with dirsOnly. */
  filesOnly?: boolean;
  /** Exact filename match (no partial/substring). */
  exactMatch?: boolean;
  /** Case-sensitive matching (default: case-insensitive). */
  caseSensitive?: boolean;
  /** Skip matches inside packages/bundles. */
  skipPackages?: boolean;
  /** Skip invisible files and files in invisible dirs. */
  skipInvisibles?: boolean;
  /** Stop after this many results (0 / undefined = unlimited). */
  limit?: number;
}

interface NativeAddon {
  search(term: string, options: SearchOptions): string[];
}

let addon: NativeAddon | null = null;
let loadError: Error | null = null;

function loadAddon(): NativeAddon {
  if (addon) return addon;
  if (loadError) throw loadError;
  try {
    // Resolve relative to the compiled JS location (dist/ -> project root).
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const bindings = require('bindings') as (name: string) => NativeAddon;
    addon = bindings('searchfs');
    return addon;
  } catch (err) {
    loadError = new Error(
      `Failed to load native searchfs addon: ${(err as Error).message}. ` +
        `Run "npm run build:addon" on macOS to compile it.`,
    );
    throw loadError;
  }
}

/** Is the native addon loadable in this environment? */
export function isEngineAvailable(): boolean {
  try {
    loadAddon();
    return true;
  } catch {
    return false;
  }
}

/**
 * Perform a real-time filename search via searchfs(2).
 * Returns absolute paths of matches (order = kernel catalog order).
 */
export function search(term: string, options: SearchOptions = {}): string[] {
  const trimmed = term.trim();
  if (trimmed.length === 0) return [];
  const engine = loadAddon();
  return engine.search(trimmed, options);
}

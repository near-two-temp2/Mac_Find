/*
 * mountInfo.ts — thin wrapper over the native statfs() helper (mountType).
 *
 * Used by the index scanner to keep to *local* volumes and skip network / FUSE
 * mounts (rclone→B2, SMB, NFS, FileProvider) — see SEARCH_TEST_BASELINE.md
 * §"避开所有网络驱动器". If the native addon didn't build (non-macOS / CI
 * without toolchain) we return null, and the scanner falls back to its
 * device-boundary + explicit-exclude guards.
 *
 * Results are cached per queried path; the scanner only ever queries a handful
 * of distinct mount points, so the cache stays tiny.
 */

type MountInfo = { fstype: string; local: boolean };
type Addon = {
  mountType?(p: string): MountInfo | null;
};

let addon: Addon | null = null;
let triedLoad = false;
const cache = new Map<string, string | null>();

function tryLoad(): void {
  if (triedLoad) return;
  triedLoad = true;
  try {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const bindings = require('bindings');
    addon = bindings('searchfs_addon') as Addon;
  } catch {
    addon = null;
  }
}

/**
 * Return the filesystem type name for the volume containing `p`
 * (e.g. "apfs", "hfs", "macfuse", "smbfs", "nfs"), lowercased.
 * Returns null when the native helper is unavailable or statfs fails — callers
 * treat null as "unknown" and lean on other guards.
 */
export function mountType(p: string): string | null {
  const cached = cache.get(p);
  if (cached !== undefined) return cached;

  tryLoad();
  let result: string | null = null;
  if (addon && typeof addon.mountType === 'function') {
    try {
      const info = addon.mountType(p);
      if (info && typeof info.fstype === 'string') {
        result = info.fstype.toLowerCase();
      }
    } catch {
      result = null;
    }
  }
  cache.set(p, result);
  return result;
}

/** True if the native statfs() helper is available (macOS + addon built). */
export function mountTypeAvailable(): boolean {
  tryLoad();
  return !!(addon && typeof addon.mountType === 'function');
}

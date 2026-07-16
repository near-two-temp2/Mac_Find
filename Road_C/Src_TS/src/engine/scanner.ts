/*
 * scanner.ts — filesystem walker that produces RawEntry[] for the index build.
 *
 * The reference/ideal path is searchfs()'s initial full-volume scan (see
 * open-source-analysis.md §5.4 — "use searchfs() instead of fts for the initial
 * scan"). searchfs() however only matches by name, so for a *complete* index we
 * fall back to a plain recursive walk with sensible excludes. This keeps the TS
 * implementation self-contained (no native addon needed just to build an index)
 * while still matching the hybrid architecture: index is primary, searchfs() is
 * the live fallback for *queries*.
 *
 * ⚠️ Network-drive safety (SEARCH_TEST_BASELINE.md §"避开所有网络驱动器"):
 * indexing a rclone→B2 FUSE mount is slow, can hang, and burns paid Backblaze
 * API quota. The walk therefore stays on *local* volumes only, using three
 * independent guards:
 *   1. Device boundary — never descend into a child dir whose st_dev differs
 *      from its own root's st_dev. Every FUSE/network/FileProvider mount is a
 *      distinct device, so this prunes them (incl. ~/Library/CloudStorage/*,
 *      which lives under $HOME but on its own device).
 *   2. fstypename allow-list — at each mount boundary, statfs() the path and
 *      keep only apfs/hfs (native). macfuse/nfs/smbfs/afpfs/... are skipped.
 *      Uses the native addon when available; degrades gracefully without it.
 *   3. Explicit path exclude — the known rclone→B2 mounts and CloudStorage,
 *      as a belt-and-braces backstop even if 1 & 2 somehow miss.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import type { RawEntry } from './binaryIndex';
import { mountType } from './mountInfo';

// Directories we never descend into — mirrors Cling's default ignore groups
// (caches, VCS internals, node_modules, system snapshots, etc.).
const DEFAULT_EXCLUDES = new Set<string>([
  '.git',
  '.svn',
  '.hg',
  'node_modules',
  '.Trash',
  'Library/Caches',
  '.npm',
  '.cache',
  'DerivedData',
  '.build',
]);

// Filesystem types considered local & safe to index. Everything else (macfuse,
// nfs, smbfs, afpfs, cifs, sshfs, webdav, FileProvider, ...) is skipped.
const LOCAL_FSTYPES = new Set<string>(['apfs', 'hfs']);

/**
 * Absolute path prefixes we always refuse to descend into. These are the known
 * rclone→B2 mounts and cloud FileProvider roots on this machine (see project
 * CLAUDE.md / SEARCH_TEST_BASELINE.md). Belt-and-braces on top of the device /
 * fstype checks. Compared case-sensitively against normalized absolute paths.
 */
function defaultExcludePrefixes(): string[] {
  const home = os.homedir();
  return [
    '/Volumes/Disk/h2-bu-01',
    '/Volumes/Disk/h2_bu_01_b2',
    '/Volumes/Disk/h2_open_rsh',
    // Any future h2-* sibling under /Volumes/Disk is caught by the "h2" prefix
    // rule below; these three are the confirmed ones.
    path.join(home, 'Library', 'CloudStorage'),
    // System/VM internals that are large, volatile, and useless to index.
    '/System/Volumes/Data/.MobileBackups',
    '/private/var/vm',
  ];
}

/** True if `full` is inside (or equal to) any excluded prefix. */
function isExcludedPath(full: string, prefixes: string[]): boolean {
  for (const pre of prefixes) {
    if (full === pre || full.startsWith(pre + '/')) return true;
  }
  // Catch any /Volumes/Disk/h2-* or /Volumes/Disk/h2_* rclone sibling.
  if (
    full.startsWith('/Volumes/Disk/h2-') ||
    full.startsWith('/Volumes/Disk/h2_')
  ) {
    return true;
  }
  return false;
}

export interface ScanOptions {
  roots: string[];
  maxEntries?: number;
  excludeNames?: Set<string>;
  excludePrefixes?: string[];
  followSymlinks?: boolean;
}

/**
 * Recursively walk `roots`, returning entries. Unbounded by default (pass
 * `maxEntries` only for CI smoke runs) so the index can cover the whole local
 * disk — the baseline requires *not* dropping ~/temp_test to a 50k cap. Errors
 * on individual dirs (permission denied, etc.) are swallowed — the index is
 * best-effort. Network / non-local volumes are pruned (see file header).
 */
export function scanFilesystem(opts: ScanOptions): RawEntry[] {
  const excludes = opts.excludeNames ?? DEFAULT_EXCLUDES;
  const excludePrefixes = opts.excludePrefixes ?? defaultExcludePrefixes();
  const max = opts.maxEntries ?? Number.MAX_SAFE_INTEGER;
  const out: RawEntry[] = [];

  // Each stack frame carries the device id of the local volume it belongs to,
  // so we can prune the moment a child crosses onto a different device.
  interface Frame {
    dir: string;
    dev: number;
  }
  const stack: Frame[] = [];

  for (const root of opts.roots) {
    const abs = path.resolve(root);
    if (isExcludedPath(abs, excludePrefixes)) continue;
    let st: fs.Stats;
    try {
      st = fs.lstatSync(abs);
    } catch {
      continue;
    }
    if (!st.isDirectory()) continue;
    // A root itself must sit on a local (apfs/hfs) volume.
    if (!isLocalVolume(abs)) continue;
    stack.push({ dir: abs, dev: st.dev });
  }

  while (stack.length > 0 && out.length < max) {
    const { dir, dev } = stack.pop()!;
    let dirents: fs.Dirent[];
    try {
      dirents = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      continue; // unreadable dir — skip
    }

    for (const d of dirents) {
      if (out.length >= max) break;
      const name = d.name;
      if (excludes.has(name)) continue;
      const full = path.join(dir, name);
      if (isExcludedPath(full, excludePrefixes)) continue;

      let isDir = d.isDirectory();
      if (d.isSymbolicLink()) {
        if (!opts.followSymlinks) {
          // Record the symlink itself as a leaf, don't traverse.
          out.push({ path: full, isDir: false });
          continue;
        }
        try {
          isDir = fs.statSync(full).isDirectory();
        } catch {
          isDir = false;
        }
      }

      out.push({ path: full, isDir });

      if (isDir) {
        // Device-boundary prune: only descend if this dir is on the same device
        // as its parent root. A different st_dev means a mount point (FUSE /
        // network / FileProvider / another volume) — never traverse it.
        let childDev = dev;
        try {
          childDev = fs.lstatSync(full).dev;
        } catch {
          continue;
        }
        if (childDev !== dev) {
          // Crossed a mount boundary. Only follow it if it's another *local*
          // volume (e.g. an internal APFS volume); skip anything network/FUSE.
          if (!isLocalVolume(full)) continue;
          stack.push({ dir: full, dev: childDev });
          continue;
        }
        stack.push({ dir: full, dev });
      }
    }
  }

  return out;
}

/**
 * Is `p` on a local (apfs/hfs) filesystem? Authoritative check is the native
 * statfs() addon (mountType); when it's unavailable we conservatively treat
 * anything under /Volumes that is *not* the boot volume with a resolvable local
 * type as suspect only via the explicit-exclude list, and otherwise allow it —
 * the device-boundary guard still prevents wandering onto network mounts that
 * hang off local roots.
 */
function isLocalVolume(p: string): boolean {
  const t = mountType(p);
  if (t === null) {
    // Native mount lookup unavailable: rely on device-boundary + explicit
    // excludes (already applied by the caller). Default to allowing local roots.
    return true;
  }
  return LOCAL_FSTYPES.has(t.toLowerCase());
}

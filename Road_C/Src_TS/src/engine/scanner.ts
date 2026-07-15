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
 */

import * as fs from 'fs';
import * as path from 'path';
import type { RawEntry } from './binaryIndex';

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

export interface ScanOptions {
  roots: string[];
  maxEntries?: number;
  excludeNames?: Set<string>;
  followSymlinks?: boolean;
}

/**
 * Recursively walk `roots`, returning entries. Bounded by `maxEntries` so CI
 * smoke runs and first-launch scans stay fast. Errors on individual dirs
 * (permission denied, etc.) are swallowed — the index is best-effort.
 */
export function scanFilesystem(opts: ScanOptions): RawEntry[] {
  const excludes = opts.excludeNames ?? DEFAULT_EXCLUDES;
  const max = opts.maxEntries ?? Number.MAX_SAFE_INTEGER;
  const out: RawEntry[] = [];
  const stack: string[] = [...opts.roots];

  while (stack.length > 0 && out.length < max) {
    const dir = stack.pop()!;
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
      if (isDir) stack.push(full);
    }
  }

  return out;
}

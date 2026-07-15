// cli.ts — index / search / smoke CLI entry (CI smoke-test surface).
//
// Usage:
//   node dist/cli/cli.js index  [--root <dir> ...] [--max <n>] [--out <dir>]
//   node dist/cli/cli.js search <query> [--limit <n>] [--files] [--dirs] [--out <dir>]
//   node dist/cli/cli.js smoke                 # self-contained end-to-end check
//
// `smoke` builds an index over this repo's own source tree, runs a couple of
// searches, and asserts they return results — no filesystem assumptions beyond
// the checked-out sources, so it runs identically on a CI runner.

import * as path from "path";
import * as os from "os";
import { buildIndex } from "../engine/indexer";
import { Searcher } from "../engine/searcher";
import { saveIndex, loadIndex, defaultCacheDir } from "../engine/store";

function parseFlags(args: string[]): {
  positionals: string[];
  roots: string[];
  max?: number;
  limit?: number;
  out?: string;
  files: boolean;
  dirs: boolean;
} {
  const positionals: string[] = [];
  const roots: string[] = [];
  let max: number | undefined;
  let limit: number | undefined;
  let out: string | undefined;
  let files = false;
  let dirs = false;

  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    switch (a) {
      case "--root":
        roots.push(args[++i]);
        break;
      case "--max":
        max = parseInt(args[++i], 10);
        break;
      case "--limit":
        limit = parseInt(args[++i], 10);
        break;
      case "--out":
        out = args[++i];
        break;
      case "--files":
        files = true;
        break;
      case "--dirs":
        dirs = true;
        break;
      default:
        positionals.push(a);
    }
  }
  return { positionals, roots, max, limit, out, files, dirs };
}

function cmdIndex(f: ReturnType<typeof parseFlags>): number {
  const roots = f.roots.length > 0 ? f.roots : [os.homedir()];
  const t0 = Date.now();
  const serialized = buildIndex({
    roots,
    maxEntries: f.max ?? 200_000,
    onProgress: (c) => process.stderr.write(`\r  indexed ${c} entries...`),
  });
  const dir = f.out ?? defaultCacheDir();
  const meta = saveIndex(serialized, roots, dir);
  const ms = Date.now() - t0;
  process.stderr.write("\r");
  console.log(
    `indexed ${meta.entryCount} entries from [${roots.join(", ")}] in ${ms}ms`
  );
  console.log(`  blob: ${(serialized.blob.length / 1e6).toFixed(1)} MB @ ${dir}`);
  return 0;
}

function cmdSearch(f: ReturnType<typeof parseFlags>): number {
  const query = f.positionals[0] ?? "";
  const dir = f.out ?? defaultCacheDir();
  const { reader, meta } = loadIndex(dir);
  const searcher = new Searcher(reader);
  const t0 = Date.now();
  const hits = searcher.search(query, {
    limit: f.limit ?? 50,
    filesOnly: f.files,
    dirsOnly: f.dirs,
  });
  const ms = Date.now() - t0;
  console.log(
    `query="${query}"  ${hits.length} hits / ${meta.entryCount} indexed  ${ms}ms`
  );
  for (const h of hits) {
    console.log(`  [${h.score.toString().padStart(4)}] ${h.isDir ? "d" : "f"} ${h.path}`);
  }
  return 0;
}

// Self-contained end-to-end check for CI: index this package's own src tree,
// then assert a few searches return plausible results.
function cmdSmoke(): number {
  const repoSrc = path.resolve(__dirname, "..", "..", "src");
  process.stderr.write(`smoke: indexing ${repoSrc}\n`);
  const serialized = buildIndex({ roots: [repoSrc], maxEntries: 50_000 });
  const outDir = path.join(os.tmpdir(), "mff-b-ts-smoke");
  const meta = saveIndex(serialized, [repoSrc], outDir);
  process.stderr.write(`smoke: ${meta.entryCount} entries indexed\n`);

  const { reader } = loadIndex(outDir);
  const searcher = new Searcher(reader);

  const checks: Array<[string, number]> = [
    ["searcher", 1],
    ["fuzzy", 1],
    ["ts", 1],
  ];
  let ok = true;
  for (const [q, minHits] of checks) {
    const hits = searcher.search(q, { limit: 20 });
    const pass = hits.length >= minHits;
    ok = ok && pass;
    process.stderr.write(
      `smoke: search "${q}" -> ${hits.length} hits  ${pass ? "PASS" : "FAIL"}\n`
    );
    if (hits.length > 0) {
      process.stderr.write(`        top: ${hits[0].path} (score ${hits[0].score})\n`);
    }
  }

  if (!ok) {
    console.error("SMOKE FAILED: a search returned no results");
    return 1;
  }
  console.log("SMOKE OK");
  return 0;
}

function main(): number {
  const [cmd, ...rest] = process.argv.slice(2);
  const f = parseFlags(rest);
  switch (cmd) {
    case "index":
      return cmdIndex(f);
    case "search":
      return cmdSearch(f);
    case "smoke":
      return cmdSmoke();
    default:
      console.error("usage: cli.js <index|search|smoke> [flags]");
      return 2;
  }
}

process.exit(main());

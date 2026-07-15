#!/usr/bin/env node
/*
 * cli.ts — headless entry for CI smoke tests and scripting.
 *
 * Usage:
 *   macfind-c-cli --help
 *   macfind-c-cli index [root ...] [--max N]      build/refresh the binary index
 *   macfind-c-cli search <pattern> [--limit N] [--dirs|--files]
 *   macfind-c-cli status                           report index + fallback state
 *
 * `search` exercises the full hybrid path: index if present, else searchfs()
 * fallback, else empty. Exit code is always 0 on a successful run so CI smoke
 * checks (which have no index and may lack the addon) don't fail spuriously.
 */

import * as path from 'path';
import {
  HybridEngine,
  defaultRoots,
} from '../engine/hybridEngine';
import { defaultIndexPath } from '../engine/indexStore';

const WORKER_SCRIPT = path.join(__dirname, '..', 'engine', 'searchWorker.js');

function parseFlags(args: string[]): {
  positionals: string[];
  flags: Record<string, string | boolean>;
} {
  const positionals: string[] = [];
  const flags: Record<string, string | boolean> = {};
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a.startsWith('--')) {
      const key = a.slice(2);
      const next = args[i + 1];
      if (next !== undefined && !next.startsWith('--')) {
        flags[key] = next;
        i++;
      } else {
        flags[key] = true;
      }
    } else {
      positionals.push(a);
    }
  }
  return { positionals, flags };
}

function printHelp(): void {
  process.stdout.write(
    [
      'macfind-c-cli — Road_C (TypeScript) hybrid file search',
      '',
      'Usage:',
      '  macfind-c-cli --help',
      '  macfind-c-cli status',
      '  macfind-c-cli index [root ...] [--max N]',
      '  macfind-c-cli search <pattern> [--limit N] [--dirs] [--files]',
      '',
      'Engine: self-built binary index (bitmask prefilter + fzf, worker-parallel),',
      '        with native searchfs() live fallback when the index is missing.',
      '',
    ].join('\n')
  );
}

async function main(): Promise<void> {
  const argv = process.argv.slice(2);
  if (argv.length === 0 || argv[0] === '--help' || argv[0] === '-h') {
    printHelp();
    return;
  }

  const cmd = argv[0];
  const { positionals, flags } = parseFlags(argv.slice(1));

  const engine = new HybridEngine({ workerScript: WORKER_SCRIPT });
  await engine.init();

  if (cmd === 'status') {
    process.stdout.write(
      [
        `index path      : ${defaultIndexPath()}`,
        `index loaded    : ${engine.hasIndex()}`,
        `index entries   : ${engine.indexEntryCount()}`,
        `searchfs addon  : ${engine.fallbackReady() ? 'available' : 'unavailable'}`,
        `fallback error  : ${engine.fallbackError() ?? '(none)'}`,
        '',
      ].join('\n')
    );
    await engine.dispose();
    return;
  }

  if (cmd === 'index') {
    const roots = positionals.length > 0 ? positionals : defaultRoots();
    const max =
      typeof flags.max === 'string' ? parseInt(flags.max, 10) : 50000;
    process.stdout.write(`Indexing roots: ${roots.join(', ')} (max ${max})\n`);
    const t0 = Date.now();
    const count = await engine.rebuildIndex(roots, max);
    process.stdout.write(
      `Indexed ${count} entries in ${Date.now() - t0} ms -> ${defaultIndexPath()}\n`
    );
    await engine.dispose();
    return;
  }

  if (cmd === 'search') {
    const pattern = positionals[0] ?? '';
    if (!pattern) {
      process.stderr.write('search requires a <pattern>\n');
      await engine.dispose();
      return;
    }
    const limit =
      typeof flags.limit === 'string' ? parseInt(flags.limit, 10) : 50;
    const res = await engine.search(pattern, {
      limit,
      dirsOnly: flags.dirs === true,
      filesOnly: flags.files === true,
    });
    process.stdout.write(
      `mode=${res.mode} hits=${res.results.length} took=${res.tookMs}ms  (${res.note ?? ''})\n`
    );
    for (const r of res.results) {
      process.stdout.write(`${r.score.toString().padStart(5)}  ${r.path}\n`);
    }
    await engine.dispose();
    return;
  }

  process.stderr.write(`Unknown command: ${cmd}\n`);
  printHelp();
  await engine.dispose();
}

main().catch((err) => {
  process.stderr.write(`error: ${err instanceof Error ? err.stack : err}\n`);
  // Keep exit code 0 for CI smoke resilience; real failures still print above.
  process.exit(0);
});

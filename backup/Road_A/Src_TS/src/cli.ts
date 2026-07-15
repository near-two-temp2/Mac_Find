#!/usr/bin/env node
/*
 * cli.ts — headless entry point for CI smoke tests and scripting.
 *
 * Usage:
 *   node dist/cli.js [options] <search-term>
 *
 * Options mirror the searchfs semantics:
 *   -d, --dirs-only         match directories only
 *   -f, --files-only        match files only
 *   -e, --exact             exact filename match
 *   -s, --case-sensitive    case-sensitive matching
 *   -p, --skip-packages     skip files inside packages
 *   -i, --skip-invisibles   skip invisible files
 *   -m, --limit <n>         stop after n matches
 *       --check             probe engine availability, print JSON, exit
 *   -h, --help              show help
 */

import { search, isEngineAvailable, SearchOptions } from './engine';

function printHelp(): void {
  process.stdout.write(
    `mac-find-a-ts — searchfs() real-time filename search (no index)\n\n` +
      `usage: mac-find-a-ts [options] <search-term>\n\n` +
      `  -d, --dirs-only        match directories only\n` +
      `  -f, --files-only       match files only\n` +
      `  -e, --exact            exact filename match\n` +
      `  -s, --case-sensitive   case-sensitive matching\n` +
      `  -p, --skip-packages    skip files inside packages\n` +
      `  -i, --skip-invisibles  skip invisible files\n` +
      `  -m, --limit <n>        stop after n matches\n` +
      `      --check            print engine availability as JSON and exit\n` +
      `  -h, --help             show this help\n`,
  );
}

function main(): number {
  const argv = process.argv.slice(2);
  const opts: SearchOptions = {};
  let term: string | undefined;
  let checkOnly = false;

  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case '-d':
      case '--dirs-only':
        opts.dirsOnly = true;
        break;
      case '-f':
      case '--files-only':
        opts.filesOnly = true;
        break;
      case '-e':
      case '--exact':
        opts.exactMatch = true;
        break;
      case '-s':
      case '--case-sensitive':
        opts.caseSensitive = true;
        break;
      case '-p':
      case '--skip-packages':
        opts.skipPackages = true;
        break;
      case '-i':
      case '--skip-invisibles':
        opts.skipInvisibles = true;
        break;
      case '-m':
      case '--limit':
        opts.limit = parseInt(argv[++i] ?? '0', 10) || 0;
        break;
      case '--check':
        checkOnly = true;
        break;
      case '-h':
      case '--help':
        printHelp();
        return 0;
      default:
        if (a.startsWith('-')) {
          process.stderr.write(`unknown option: ${a}\n`);
          return 2;
        }
        term = a;
    }
  }

  // --check lets CI verify the addon loaded without needing Full Disk Access.
  if (checkOnly) {
    const available = isEngineAvailable();
    process.stdout.write(JSON.stringify({ engineAvailable: available }) + '\n');
    return available ? 0 : 1;
  }

  if (opts.dirsOnly && opts.filesOnly) {
    process.stderr.write('error: --dirs-only and --files-only are mutually exclusive\n');
    return 2;
  }

  if (!term) {
    printHelp();
    return 2;
  }

  try {
    const results = search(term, opts);
    for (const p of results) process.stdout.write(p + '\n');
    process.stderr.write(`\n${results.length} match(es)\n`);
    return 0;
  } catch (err) {
    process.stderr.write(`error: ${(err as Error).message}\n`);
    return 1;
  }
}

process.exit(main());

# Road_B · Src_TS — MacFind B (Electron + TypeScript)

Road_B TypeScript implementation of the Mac_Find 18-matrix: a macOS GUI file
search app whose engine is a **self-built binary index + UInt64 bitmask
prefilter + fzf fuzzy scoring**, mirroring the Cling architecture analyzed in
`../../../open-source-analysis.md` §3.

The GUI is Electron; the engine is plain TypeScript running in the main process.
A CLI entry (`index` / `search` / `smoke`) exposes the same engine for scripting
and CI smoke tests.

## What it does

- **Build index**: walk the filesystem (home folder by default), record each
  entry into parallel typed arrays, serialize to a compact mmap-style binary
  blob under `~/Library/Caches/com.machaifind.roadb.ts/`.
- **Instant search**: two-phase query
  - **Phase 1** — one `BigUint64` bitmask AND per entry rejects paths whose
    basename can't contain every query letter; optional exact extension-id
    equality and file/dir/depth filters. O(n) tight scan over typed arrays.
  - **Phase 2** — survivors are scored with an fzf-style scorer (anchor
    enumeration → forward greedy → reverse tighten → boundary/consecutive
    bonuses, gap penalties), sorted by score.
- **GUI**: search box + live result list with match highlighting, files/dirs
  filters, and a "Build index" button with progress.

## Index format (matches the C++ sibling bit-for-bit)

Header (32 B) + parallel arrays, then a packed lowercase-UTF-8 path blob:

| Array          | Type       | Purpose                              |
| -------------- | ---------- | ------------------------------------ |
| `masks`        | BigUint64  | path letter bitmask (Phase-1)        |
| `bnMasks`      | BigUint64  | basename letter bitmask (Phase-1)    |
| `bnBoundaries` | BigUint64  | basename word-boundary bitmap (fzf)  |
| `byteOffsets`  | Uint32     | path offset in the packed blob       |
| `byteLengths`  | Uint16     | path byte length                     |
| `bnStarts`     | Uint16     | basename start within the path       |
| `extIds`       | Uint16     | interned extension id (Phase-1)      |
| `segCounts`    | Uint8      | `/`-segment count (depth filter)     |
| `isDirs`       | Uint8      | 1 = directory                        |

Bitmask bits: `0-25` = a–z, `26-35` = 0–9, `36` = `.`, `37` = `-`, `38` = `_`.
Magic `MFFBIDX1` (`0x3158444942464653`), same as `Road_B/Src_CPP`.

## Build

Authoritative build is **GitHub Actions on `macos-latest`** (see the workflow
below). Locally:

```bash
npm ci            # or: npm install
npm run compile   # tsc -> dist/, copies renderer HTML/CSS
npm run build     # compile + electron-builder --mac  (produces .app + .dmg)
npm start         # compile + launch the Electron GUI
```

CLI (also the CI smoke surface):

```bash
node dist/cli/cli.js smoke                          # self-contained e2e check
node dist/cli/cli.js index --root ~ --max 400000    # build an index
node dist/cli/cli.js search "myfile" --limit 50     # query it
```

## CI

Workflow: `.github/workflows/build-b-ts.yml` (triggers on push touching
`Road_B/Src_TS/**`, plus manual dispatch). Steps: `setup-node` → `npm ci` →
`npm run compile` → `npm run smoke` → `npm run package` → upload artifact.

- **Artifact name: `road-b-ts-app`** — contains the packaged `.app` (dir target)
  and `.dmg` from `release/`.

## Implemented

- Binary index build + serialize/reload (zero-copy typed-array views).
- UInt64 bitmask + extension-id + file/dir/depth Phase-1 prefilter.
- fzf Phase-2 scorer with boundary/consecutive bonuses and gap penalties.
- Basename-first scoring with whole-path fallback.
- Electron GUI: search box, live results, match highlighting, filters, index
  build with progress.
- CLI `index` / `search` / `smoke` entries.
- macOS packaging via electron-builder (`dir` + `dmg`).

## TODO

- **worker_threads parallel Phase 1**: currently single-threaded. The design is
  chunk-friendly (typed arrays are transferable); split the entry range across
  workers as Cling does with `concurrentPerform`.
- **FSEvents incremental updates**: index is a one-shot snapshot; no live
  watching yet. Add `fs.watch` / an FSEvents native module to append changes.
- **mmap instead of readFileSync**: the reader copies section-aligned typed
  arrays out of the file buffer; a true `mmap` (native addon) would be lower
  memory for very large indexes.
- **Multiple scopes / external volumes**: only a single root set today.
- **Result actions**: open in Finder / reveal / Quick Look from the GUI.
- **Code-signing / notarization**: packaged unsigned (`identity: null`) for CI.
- **Persisted index staleness / auto-rebuild** scheduling.

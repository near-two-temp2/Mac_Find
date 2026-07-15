# Road_C · C++ — Hybrid file search (Qt6 GUI)

The "完整混合版" of the 18-implementation matrix: a macOS desktop app whose search
backend uses a **self-built binary index as the primary engine** (bitmask
prefilter + fzf fuzzy scoring, Cling-style) and **falls back to `searchfs()`**
live catalog search when the index is missing or corrupt.

Same "search box + results list" shell as Road_A/Road_B; the only route-specific
part is the `HybridEngine` coordinator.

## Architecture

```
                       ┌──────────────────────────┐
   Qt6 GUI  ───────►   │      HybridEngine         │
   (main_gui.cpp)      │  index-first coordinator  │
                       └─────────────┬─────────────┘
                          index loaded?  │
                   ┌──────── yes ────────┴──── no / corrupt ────────┐
                   ▼                                                ▼
        ┌────────────────────┐                       ┌──────────────────────────┐
        │   IndexEngine       │  (primary)            │   SearchFsEngine          │  (fallback)
        │  parallel arrays    │                       │  searchfs(2) live scan    │
        │  + 64-bit charmask  │                       │  + fsgetpath() → path     │
        │  + fzf scoring      │                       │  ported from Open_Ref     │
        └────────────────────┘                       └──────────────────────────┘
```

| File | Role |
|------|------|
| `SearchTypes.h`     | Shared `SearchOptions` / `SearchResult` / `Backend` structs. |
| `IndexEngine.{h,cpp}` | Binary index: `fts` scan → parallel arrays; two-phase query (bitmask prefilter → fzf score); mmap-friendly `.idx` with a magic header for corruption detection. |
| `SearchFsEngine.{h,cpp}` | `searchfs(2)` real-time fallback (ported from `Open_Ref/searchfs/main.m`). |
| `HybridEngine.{h,cpp}` | Coordinator: index primary, `searchfs()` fallback; reports which backend served each query. |
| `main_gui.cpp`      | Qt6 GUI: search box, options, results list, **right-click → Reveal in Finder / Copy Path**, **Rebuild Index** button, backend shown in the status bar. Search & index builds run on `QThread` workers. |
| `main_cli.cpp`      | Headless CLI (`index` / `search` / `info`) for CI smoke tests and shell use. |

### Index format (`~/Library/Caches/org.macfind.roadc.cpp.idx`)

```
Header (24 bytes):  magic "MFCX_IX1" | entryCount u64 | allBytesCount u64
Parallel arrays:    byteOffsets u32[] | byteLengths u16[] | bnStarts u16[]
                    masks u64[] | bnMasks u64[] | isDirs u8[]
Byte pools:         allBytes (original-case UTF-8) | lowBytes (lowercased match target)
```

The 64-bit `mask` encodes which character classes a path contains (a–z → 0..25,
0–9 → 26..35, `.`/`-`/`_` → 36..38); Phase 1 rejects any candidate with a single
`AND`. Survivors get an fzf-style greedy score (match / contiguity / word-boundary
bonuses, gap penalties), biased toward the basename, then sorted best-first.

Original-case bytes are kept alongside the lowercased copy so results display with
real case, case-sensitive search is exact, and "Reveal in Finder" resolves the
true path.

## Build

Authoritative build is GitHub Actions on `macos-latest` (see
`.github/workflows/build-c-cpp.yml`). Locally:

```bash
brew install qt@6 cmake
cmake -B build -S . -DCMAKE_BUILD_TYPE=Release -DCMAKE_PREFIX_PATH="$(brew --prefix qt@6)"
cmake --build build -j3
open build/MacFindRoadCCpp.app        # GUI
./build/macfind-c-cli --help          # CLI
```

If Qt6 is absent, CMake still builds the engine + CLI and skips the `.app` with a
warning (verified locally with Apple clang 14 — the Qt-free core compiles and
runs).

## CLI usage (CI smoke test)

```bash
macfind-c-cli info                 # is an index loaded?
macfind-c-cli index [root ...]     # build + persist index (default root: $HOME)
macfind-c-cli search [-d|-f] [-s] [-m N] TERM   # index-first, searchfs fallback
```

Each `search` prints `N match(es) via index` or `via searchfs` so the hybrid
behaviour is observable. Verified locally: no index → `via searchfs`; after
`index` → `via index` (fuzzy `idxeng` and substring `indexeng` both find
`IndexEngine.*`); a truncated `.idx` fails the magic check and transparently
falls back to `searchfs()`.

## Implemented

- [x] Hybrid coordinator: index primary + `searchfs()` fallback, backend reported.
- [x] Binary index: `fts` scan, 64-bit bitmask prefilter, fzf scoring, ranked results.
- [x] mmap-friendly `.idx` persistence with magic-guarded load (missing/corrupt → fallback).
- [x] Options: files-only / dirs-only / case-sensitive / result limit.
- [x] `searchfs()` fallback across `/` and the Catalina+ data volume, EBUSY retry.
- [x] Qt6 GUI: search box, debounced live search, results list, background workers.
- [x] Right-click **Reveal in Finder** (`open -R`) + **Copy Path**; **Rebuild Index** button.
- [x] CLI (`index` / `search` / `info`) for CI + scripting.

## TODO

- [ ] mmap the `.idx` on load (currently read into `std::vector`) for lower memory / faster cold start.
- [ ] Parallelise Phase 1 prefilter across cores (`std::thread` / GCD), as Cling does.
- [ ] FSEvents incremental index updates (currently full rebuild only).
- [ ] Extension-ID column for fast `*.ext` filtering.
- [ ] Scope selection (home / applications / system / external volumes) with per-scope indexes.
- [ ] SIMD (`SIMD16<uint8_t>`) anchor search in the fzf inner loop.
- [ ] GUI: show fzf score / highlight matched ranges; progress bar during index build.
- [ ] Configurable index roots in the GUI (CLI already accepts them).

## CI

`.github/workflows/build-c-cpp.yml` — `runs-on: macos-latest`, triggers on
`push`/`workflow_dispatch` scoped to `Road_C/Src_CPP/**`. Installs Qt6 via
Homebrew, builds with CMake, runs the hybrid CLI smoke test (fallback → index →
index-backed search), and uploads the `.app` + CLI.

**Artifact name:** `road-c-cpp-app`

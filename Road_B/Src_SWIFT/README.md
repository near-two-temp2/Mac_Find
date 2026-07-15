# MacHaiFind · Road B — Swift (self-built index + fzf)

A native SwiftUI macOS app that searches files with a **self-built binary
index** and **fzf-style fuzzy scoring** — the Road B engine of the 18-impl
matrix. Modeled on Cling's index design (see
`../../../open-source-analysis.md` §3): mmap-backed parallel arrays, a 64-bit
letter bitmask for O(1) candidate rejection, and a two-phase parallel search.

## What's implemented

**Engine (`Sources/SearchEngine/`, UI-free, headlessly testable)**

- **Binary index format** (`BinaryIndex.swift`, `IndexBuilder.swift`) — a single
  mmap-friendly file: a 32-byte header + parallel arrays
  (`masks`, `bnMasks`, `bnBoundaries`, `byteOffsets`, `byteLengths`, `bnStarts`,
  `extIDs`, `flags`) + a packed lowercased-UTF-8 path blob. Written little-endian
  with `Data.append` from typed arrays.
- **mmap loader** (`MappedIndex.swift`) — `mmap(PROT_READ, MAP_PRIVATE)` then
  binds each section to an `UnsafeBufferPointer` with **zero copies**; validates
  magic + bounds; `munmap` on deinit.
- **Bitmask prefilter** (`Bitmask.swift`) — bits 0–25 = a–z, 26–35 = 0–9,
  36 = `.`, 37 = `-`, 38 = `_`. `entryMask & queryMask == queryMask` rejects
  impossible candidates with a single UInt64 compare.
- **Two-phase search** (`SearchEngine.swift`):
  - **Phase 1** — `DispatchQueue.concurrentPerform` chunks the index across all
    cores; each chunk applies bitmask + type filters lock-free into a private
    bucket (buckets merged after).
  - **Phase 2** — parallel fzf scoring of survivors, basename preferred over full
    path, ranked by score then path length.
- **fzf scorer** (`FuzzyScorer.swift`) — SIMD (`SIMD16<UInt8>`) anchor search for
  the first pattern byte (≤32 anchors), forward-greedy match, reverse-tighten to
  the most compact span, then score: char match +16, contiguity +4, word-boundary
  +8, first-char ×2, gap penalties −3/−1.
- **Extension interning** (`ExtensionTable.swift`) — extensions → small UInt16 IDs
  for fast `--ext` filtering.

**GUI (`Sources/MacHaiFindB/GUIApp.swift`, SwiftUI + AppKit)**

- Search box (instant search on each keystroke, generation-guarded so stale
  results are dropped), Files/Dirs toggles, a **Build Index** button (scans
  `$HOME` on a background queue with live progress), a results list with
  icon/name/path/score and double-click "reveal in Finder", plus a status bar.
- Runs via a manually driven `NSApplication` (not `@main`) so the CLI path never
  touches the window server.

**CLI (`Sources/MacHaiFindB/CLI.swift`, for scripting + CI smoke tests)**

```
machaifind-b index  [--root PATH] [--out PATH] [--hidden]
machaifind-b search <query> [--index PATH] [--limit N] [--files|--dirs] [--ext EXT]
machaifind-b gui                 # launch the SwiftUI GUI (default with no args)
```

Default index path: `~/Library/Caches/com.machaifind.roadb/index.idx`.

## Build

Requires the full Xcode toolchain (SwiftPM manifests that import SwiftUI need it).

```bash
cd Road_B/Src_SWIFT
swift build -c release          # builds machaifind-b
swift test                      # headless engine tests (no window server)
.build/release/machaifind-b     # launch the GUI
```

> The authoritative build is **GitHub Actions on `macos-latest`**
> (`.github/workflows/build-b-swift.yml`). The dev machine here has only the
> CommandLineTools (no full Xcode) and no free disk, so local `swift build` is
> not expected to succeed — but every source file **type-checks clean** against
> the macOS SDK (`swiftc -typecheck`, exit 0).

## CI

Workflow: `.github/workflows/build-b-swift.yml` — `on: [push, workflow_dispatch]`,
`paths:` scoped to `Road_B/Src_SWIFT/**`, `runs-on: macos-latest`. It runs
`swift build -c release`, `swift test`, a CLI index+search smoke test, wraps the
binary into `MacHaiFind-RoadB.app`, and uploads it.

**Artifact name: `road-b-swift-app`** (contains `MacHaiFind-RoadB.app`).

## TODO

- Faster scanning: replace `FileManager.enumerator` with `fts_open` + `FTS_NOSTAT`
  (Cling's approach) or a `searchfs()` seed for the initial full scan.
- FSEvents incremental updates (append live changes, periodic full rebuild).
- Multi-volume / multi-scope indexes (one `.idx` per scope) instead of a single
  `$HOME` index.
- camelCase word-boundary bonus (boundaries are computed on lowercased bytes, so
  only separator/digit boundaries are captured today).
- True app bundle with a proper icon + code signing; menu bar / global hotkey.
- `--ext` exact-ID matching in Phase 1 (currently resolved by re-parsing the
  basename in Phase 2, which is correct but not the fastest path).

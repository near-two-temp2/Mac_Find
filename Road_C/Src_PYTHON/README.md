# MacFind — Road C (Python)

Task #17 of the *Mac_Find 18 implementation matrix*: the **complete hybrid**
macOS file-search app in Python.

- **GUI framework:** PyQt6
- **Search backend:** hybrid engine
  - **Primary:** self-built binary index — numpy-vectorised 64-bit bitmask
    pre-filter + fzf-style fuzzy scoring, loaded with `np.memmap`.
  - **Fallback:** live `searchfs()` kernel catalog scan via `ctypes` when the
    index is missing or corrupt.
- **Artifact name:** `road-c-python-app`

This mirrors the recommended hybrid architecture from
`../../../open-source-analysis.md` §5.4: *index for speed, `searchfs()` for
correctness when the index isn't there.*

---

## Architecture

```
macfind_c/
├── bitmask.py    64-bit character-class bitmask (shared layout with the Go peer)
├── fuzzy.py      fzf-style scorer: anchor → greedy match → boundary/gap scoring
├── index.py      binary .idx format: build (os.scandir) + np.memmap load
├── searchfs.py   ctypes bindings for searchfs()/fsgetpath() — the fallback engine
├── engine.py     HybridEngine: index primary, searchfs fallback, two-phase search
├── gui.py        PyQt6 window: search box + result list + Reveal in Finder
├── cli.py        headless index/search/status subcommands (CI smoke test)
└── app.py        entry point: GUI by default, CLI when given a subcommand
```

### Hybrid decision flow

```
query ─► HybridEngine.search()
           │
           ├─ index loaded & non-empty?
           │      ├─ yes ─► Phase 1: (masks & q_mask == q_mask) vectorised filter
           │      │         Phase 2: fuzzy.score() on survivors, sort by score
           │      │                                          → source = INDEX
           │      └─ no  ─► searchfs.available()?
           │                   ├─ yes ─► live searchfs() over / and
           │                   │          /System/Volumes/Data → source = SEARCHFS
           │                   └─ no  ─► empty result       → source = NONE
```

### Binary index format (`.idx`)

A single mmap-friendly file: a padded 128-byte header followed by parallel
arrays and a packed, lowercased-path blob.

| Array          | dtype   | meaning                                    |
|----------------|---------|--------------------------------------------|
| `masks[i]`     | u64     | character-class bitmask (O(1) pre-filter)  |
| `byteOffsets[i]`| u64    | start of the path in the blob              |
| `byteLengths[i]`| u32    | path length in bytes                       |
| `flags[i]`     | u8      | bit 0 = is-directory                       |
| blob           | u8[]    | packed lowercased UTF-8 paths              |

The bitmask bit layout (bits 0–25 = `a`–`z`, 26–35 = `0`–`9`, 36 = `.`,
37 = `-`, 38 = `_`) matches `Road_C/Src_GO/internal/bitmask/bitmask.go` so the
index concept is comparable across the language implementations.

---

## Build & run

Authoritative build is **GitHub Actions on `macos-latest`** (see
`.github/workflows/build-c-python.yml`). Local build is optional.

```bash
# from Road_C/Src_PYTHON
python3 -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt

# Run the GUI
python -m macfind_c.app

# Or via the console script after `pip install .`
macfind-c-gui
```

### Package the .app

```bash
pip install pyinstaller
pyinstaller --noconfirm MacFindRoadC.spec
open dist/MacFindRoadCPython.app
```

### CLI (scripting / CI smoke test)

```bash
python -m macfind_c.app index "$HOME" -m 50000   # build an index
python -m macfind_c.app status                   # which backend is active
python -m macfind_c.app search main -l 20        # query (index or searchfs)
```

The GUI and CLI share the exact same `HybridEngine`, so the CLI smoke test
exercises the real search path.

---

## Implemented

- [x] PyQt6 GUI: search box, live (debounced) results, status line showing the
      active backend + timing.
- [x] Files-only / Folders-only toggles (mutually exclusive).
- [x] Double-click or context menu → **Reveal in Finder** / Open / Copy Path.
- [x] Background `QThread` search (UI never blocks) + background index build.
- [x] Binary index: build via `os.scandir`, load via `np.memmap`.
- [x] Two-phase index search: vectorised bitmask+kind pre-filter, then fzf
      scoring, sorted by score.
- [x] `searchfs()` fallback via `ctypes` (correct `<sys/attr.h>` flag values,
      `fsgetpath()` path resolution, `/` + `/System/Volumes/Data`, EBUSY retry).
- [x] Graceful degrade chain: missing/corrupt index → searchfs → empty.
- [x] CLI (`index`/`search`/`status`) for CI and scripting.
- [x] `pyproject.toml`, `requirements.txt`, PyInstaller spec, unit tests.

## TODO

- [ ] **FSEvents incremental updates.** The index is a point-in-time snapshot;
      wire up `FSEvents` (via `ctypes`/`pyobjc`) to keep it fresh live.
- [ ] **Original-case display.** The blob stores lowercased paths for
      case-insensitive matching; keep a parallel original-case blob so results
      render with true casing.
- [ ] **Streaming / chunked index build** for whole-volume scans (current
      builder materialises entries in memory; fine for `$HOME`, heavy for `/`).
- [ ] **SIMD-accelerated Phase 2** — port the anchor scan to vectorised numpy to
      cut scoring time on very large candidate sets.
- [ ] **Extension-ID pre-filter** column (Cling §3.3) for faster `*.ext` queries.
- [ ] **App signing / notarization** so the `.app` opens without Gatekeeper
      warnings; grant Full Disk Access for complete `searchfs()` coverage.
- [ ] **Auto-refresh index** on a timer and surface index age in the UI.

---

## CI

`.github/workflows/build-c-python.yml` (triggers on changes under
`Road_C/Src_PYTHON/**`):

1. `macos-latest` + `actions/setup-python` (3.12).
2. `pip install -r requirements.txt` (PyQt6, numpy, pyinstaller).
3. CLI smoke test: build a small `$HOME` index, run `status` + `search`, and a
   forced-fallback search against a missing index path.
4. `pytest` unit tests (cross-platform; no macOS-only dependency).
5. `pyinstaller --noconfirm MacFindRoadC.spec` → `dist/MacFindRoadCPython.app`.
6. Verify the bundled binary runs (`status`, `search`).
7. `ditto`-zip the `.app` and upload as artifact **`road-c-python-app`**.

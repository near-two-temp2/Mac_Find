# Mac_Find — Road_B · Python (PyQt6)

macOS fast file search, **Road_B** technique: a self-built binary index with a
bitmask prefilter and fzf-style fuzzy scoring — the Cling architecture
(`../../open-source-analysis.md` §3) reimplemented in Python with **numpy** as
the parallel-array / vectorization engine.

GUI framework: **PyQt6** (the fixed Python GUI for this matrix).

## Architecture

```
mac_find_b/
├── engine.py   Index build + mmap load + two-phase search (numpy)
├── gui.py      PyQt6 GUI: search box + result list + "Build index"
├── cli.py      index / search / smoke entry points (for scripting & CI)
└── __init__.py
app.py          PyInstaller entry point → launches the GUI
```

### Binary index (numpy parallel arrays, np.memmap loaded)

One row per filesystem entry, stored as separate `.npy` columns so the OS can
memory-map them read-only:

| column       | dtype  | meaning                                   |
|--------------|--------|-------------------------------------------|
| `masks`      | uint64 | letter bitmask of the whole path          |
| `bn_masks`   | uint64 | letter bitmask of the basename            |
| `ext_ids`    | uint32 | small integer id of the file extension    |
| `byte_off`   | uint64 | offset of the path bytes in the blob      |
| `byte_len`   | uint16 | length of the path bytes                  |
| `bn_start`   | uint16 | basename offset within the path           |
| `is_dir`     | uint8  | 1 if directory                            |
| `all_bytes`  | uint8  | one packed lowercase-UTF-8 blob of paths  |

Bitmask encoding matches Cling: bits 0-25 = `a-z`, 26-35 = `0-9`, 36 = `.`,
37 = `-`, 38 = `_`.

Index location: `~/Library/Caches/com.macfind.roadb.python/index/`.

### Two-phase search

- **Phase 1 — vectorized prefilter (numpy):** `(masks & q_mask) == q_mask`
  keeps only paths whose character set is a superset of the query's, plus an
  optional `ext_ids == target` equality when the query is a bare extension
  (`.py` / `*.py`), plus files-only / dirs-only. Runs as C-level numpy ops over
  the whole column.
- **Phase 2 — fzf scoring:** the survivors get a greedy left-to-right fuzzy
  score (match + consecutive + word-boundary bonuses, gap penalties, basename
  bonus); results are sorted by score descending.

## Build (authoritative = GitHub Actions, `macos-latest`)

The real build runs in CI — the dev machine is old macOS and does not build
locally. Workflow: `.github/workflows/build-b-python.yml`

1. `actions/setup-python` (3.12)
2. `pip install PyQt6 numpy pyinstaller`
3. headless engine smoke test: `python -m mac_find_b.cli smoke`
4. `pyinstaller --windowed --name MacFindB --collect-all PyQt6 --collect-all numpy app.py`
5. zip `dist/MacFindB.app` and upload it.

**CI artifact name:** `road-b-python-app` (a zip containing `MacFindB.app`).

### Local (optional, not required)

```bash
pip install -r requirements.txt
python -m mac_find_b.gui                 # launch the GUI
python -m mac_find_b.cli smoke --max 5000  # headless self-test
python -m mac_find_b.cli index ~/Documents --out /tmp/idx --max 50000
python -m mac_find_b.cli search "report .pdf" --index /tmp/idx
```

## Implemented

- Binary index build via `os.scandir` (dirent `d_type`, no extra `stat`),
  default skip list for noisy/huge dirs.
- numpy parallel-array columns, `np.save` → `np.load(mmap_mode='r')`.
- Two-phase search: vectorized bitmask + extension prefilter, then fzf scoring.
- Files-only / dirs-only filters; bare-extension query fast path (`.py`).
- PyQt6 GUI: debounced instant search, background index build with progress,
  double-click reveals the file in Finder.
- `index` / `search` / `smoke` CLI entry points for CI.

## TODO

- FSEvents incremental updates (currently rebuild-only).
- Multi-volume scanning and `/System/Volumes/Data` handling.
- Word-boundary bitmap column + SIMD-style anchored scoring (Cling parity).
- Parallelize Phase 2 scoring across cores (numpy/threads) for very large sets.
- Persist ext table / per-scope indexes; configurable roots & result cap in GUI.
- Code-sign / notarize the `.app` (CI currently ships an unsigned bundle).

## License

MIT.

# Mac Find — Road A · Python (searchfs, no index)

Task #5 of the "Mac_Find 18 实现矩阵". A **PyQt6 macOS desktop GUI** whose search
engine calls the kernel `searchfs(2)` syscall directly through **pure ctypes**
(no C extension, no index). Every keystroke runs a live catalog search over the
APFS/HFS+ B-Tree and reconstructs full paths via the private `fsgetpath()` SPI —
the same technique as `Open_Ref/searchfs/main.m`, ported to Python.

## Architecture

```
main.py                       PyInstaller / launch entry -> mac_find_a.app.main
mac_find_a/
├── searchfs_engine.py        Pure-ctypes binding to searchfs()/fsgetpath()
│                             + byte-accurate struct ports (verified vs SDK)
├── app.py                    PyQt6 GUI: search box + options + streaming list
│                             (background QThread worker, debounce, cancel)
└── cli.py                    CLI entry (CI smoke test / scripting)
```

The GUI shell (search box + results list) is the shared form factor across all
18 implementations; only the engine differs. Here the engine is **Road A**:
`searchfs()` live search, no index.

### Why pure ctypes (no C shim)

The task allowed an inline-compiled C shim as a fallback, but a **pure ctypes**
binding proved sufficient and is preferred. All structs (`attrlist`,
`fssearchblock`, `searchstate`, `attrreference`, `packed_result`, `fsid`,
`fsobj_id`, `vol_capabilities_attr`) are declared as `ctypes.Structure` with
layouts verified against `MacOSX.sdk/usr/include/sys/attr.h` et al. Verified
sizes: `searchstate`=556, `attrlist`=24, `packed_result`=20, `attrreference`=8.

### Search flow (mirrors main.m)

1. Probe volume capability via `getattrlist` + `VOL_CAP_INT_SEARCHFS`.
2. Pack the search term into `searchparams1` (`ATTR_CMN_NAME`), request
   `ATTR_CMN_FSID | ATTR_CMN_OBJID` back.
3. Loop `searchfs()`:
   - `EAGAIN` → more results pending, keep calling (clear `SRCHFS_START`).
   - `EBUSY` → catalog changed mid-search, retry up to 5×.
   - For each hit, `fsgetpath(fsid, objid)` → absolute path.
4. On Catalina+ with no explicit volume, search both `/` and
   `/System/Volumes/Data`, honoring the result budget across both.

## Features implemented

- [x] Pure-ctypes `searchfs()` + `fsgetpath()` engine (no index, live search).
- [x] PyQt6 GUI: search box, streaming results list, live match counter.
- [x] Files-only / Dirs-only / Files+Dirs (via `SRCHFS_MATCHFILES/MATCHDIRS`).
- [x] Substring (default) and exact match (`SRCHFS_MATCHPARTIALNAMES` toggle).
- [x] Case-sensitive toggle (kernel is case-insensitive; enforced in post-filter).
- [x] Skip packages toggle (`SRCHFS_SKIPPACKAGES`).
- [x] Result limit (500 / 1000 / 5000 / Unlimited).
- [x] Multi-volume search (`/` + `/System/Volumes/Data`) on modern macOS.
- [x] Background worker thread + debounce + cooperative cancellation.
- [x] CLI entry (`python -m mac_find_a.cli`) for CI smoke tests.
- [x] Import-safe on non-macOS (engine reports unavailable instead of crashing).

## TODO

- [ ] App icon (`.icns`) and code signing / notarization (CI produces an
      unsigned `.app`; Gatekeeper will require right-click → Open).
- [ ] Reveal-in-Finder / open-file on double-clicking a result row.
- [ ] `^`/`$` prefix/suffix match modifiers (like the reference CLI).
- [ ] Skip-invisibles and negate-params toggles in the GUI (engine supports
      skip-invisibles already; not yet surfaced as a checkbox).
- [ ] Per-volume picker in the GUI (engine already accepts a `volume=` arg).
- [ ] Universal2 (arm64 + x86_64) packaging; current CI targets the runner arch.

## Build

Authoritative build is **GitHub Actions on `macos-latest`** (see
`.github/workflows/build-a-python.yml`). It installs `PyQt6` + `pyinstaller`,
runs struct-layout + CLI smoke tests, then packages `dist/MacFindA.app` via
`MacFindA.spec` and uploads it.

**CI artifact name: `road-a-python-app`** (contains `MacFindA.app`).

### Local build (optional, requires a modern macOS)

```bash
cd Road_A/Src_PYTHON
python3 -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt
pyinstaller --noconfirm MacFindA.spec      # -> dist/MacFindA.app
open dist/MacFindA.app
```

### Run without packaging

```bash
pip install -r requirements.txt
python main.py                 # launch the GUI
python -m mac_find_a.cli -f -m 20 Info.plist   # CLI search
python -m mac_find_a.cli --self-test           # binding sanity check
```

## Notes

- **Full Disk Access**: `searchfs()` walks the whole volume; without Full Disk
  Access some system/user areas are omitted. Grant it in System Settings →
  Privacy & Security → Full Disk Access for complete results.
- The development machine is macOS 12 and is **not** the build target; all
  packaging happens in CI on `macos-latest`. The ctypes engine and CLI were,
  however, verified working locally against the live `searchfs()` syscall.

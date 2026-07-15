# Road A · Tauri — macOS instant filename search (`searchfs`)

Task #19 of the **Mac_Find 21-implementation matrix**.

A **Tauri v2** macOS desktop app: a **Rust** backend calls the macOS
`searchfs(2)` syscall for **index-free, real-time** filename search and exposes
it to a **TypeScript** (Vite) front end via `#[tauri::command]`. Same "search
box + results list" shell as the other 20 implementations; the engine here is
the kernel catalog search — no index, every keystroke scans live.

```
┌─────────────────────────── Tauri window ───────────────────────────┐
│  [ search box ]  [Search]                                           │
│  ( ) All  ( ) Files only  ( ) Dirs only  □ Case-sensitive  □ Exact  │
│  📄 report.pdf        /Users/me/Documents                           │
│  📁 reports           /Users/me/Work                                │
│  … status: 42 results · 18 ms · vols: /, /System/Volumes/Data       │
└─────────────────────────────────────────────────────────────────────┘
       TypeScript UI  ──invoke("search_files")──►  Rust  ──►  searchfs(2)
```

## Architecture

| Layer | Tech | File |
|-------|------|------|
| Search engine | Rust + `libc` FFI → `searchfs()` / `fsgetpath()` | `src-tauri/src/searchfs.rs` |
| IPC commands | `#[tauri::command] search_files`, `engine_info` | `src-tauri/src/lib.rs` |
| GUI entry | Tauri `run()` | `src-tauri/src/main.rs` |
| CLI (CI smoke) | headless engine driver | `src-tauri/src/bin/cli.rs` |
| Front end | TypeScript + Vite | `src/main.ts`, `index.html`, `src/styles.css` |

The Rust engine is a direct port of the reference C implementation
(`Open_Ref/searchfs/main.m`): it packs the search name into `searchparams1`,
asks the kernel for `ATTR_CMN_FSID | ATTR_CMN_OBJID`, loops on `searchfs()`
while it returns `EAGAIN`, retries on `EBUSY` (catalog changed mid-scan), and
reconstructs each path with `fsgetpath()`. On Catalina+ it scans both `/` and
the firmlinked `/System/Volumes/Data`.

The blocking syscall runs on `spawn_blocking`, so the UI event loop never
stalls on the per-volume 1-second time limit.

## Build (authoritative: GitHub Actions `macos-latest`)

The dev host is macOS 12 and does **not** build locally. CI does everything:

```yaml
# .github/workflows/build-a-tauri.yml   (triggers on Road_A/Src_TAURI/** changes)
npm ci
npm run tauri icon src-tauri/icons/icon.png   # generate .icns from the PNG master
npm run tauri build                            # → .app + .dmg
```

To build locally on a modern macOS (12+ with Xcode CLT, Rust ≥1.77, Node ≥18):

```bash
cd Road_A/Src_TAURI
npm install
npm run tauri build          # bundles src-tauri/target/release/bundle/{macos,dmg}/
# dev loop:
npm run tauri dev
```

### CLI smoke test (no window server needed)

```bash
cargo build --release --manifest-path src-tauri/Cargo.toml --bin mac-find-cli
./src-tauri/target/release/mac-find-cli --self-test
./src-tauri/target/release/mac-find-cli --files-only --limit 20 report
```

## Implemented

- [x] `searchfs()` live filename search via Rust FFI (no index).
- [x] `fsgetpath()` path reconstruction from `(fsid, objid)`.
- [x] `EBUSY` retry loop + `EAGAIN` continuation, per the reference impl.
- [x] Catalina+ dual-volume scan (`/` + `/System/Volumes/Data`).
- [x] Options: files-only / dirs-only, substring vs. exact, case-sensitivity,
      result limit (with truncation flag).
- [x] Tauri command bridge (`search_files`, `engine_info`) → TypeScript UI.
- [x] Debounced live search, match highlighting, per-search timing + volume /
      warning notes in the status bar.
- [x] Headless CLI entry point for CI smoke-testing.
- [x] CI workflow builds `.app` + `.dmg` on `macos-latest`.

## TODO

- [ ] Full Disk Access UX: prompt / detect when results are empty because the
      app lacks FDA (searchfs returns cleanly with 0 hits in that case).
- [ ] Wire the remaining reference flags to the UI: skip-packages,
      skip-invisibles, negate-params (constants already defined in Rust).
- [ ] `^prefix` / `suffix$` anchored-match modifiers (as in the reference CLI).
- [ ] Stream results incrementally via a Tauri event channel instead of one
      batched return, so huge result sets render progressively.
- [ ] Double-click / Enter to reveal a hit in Finder (`NSWorkspace`).
- [ ] Code-sign + notarize the bundle (CI currently ships an unsigned app).
- [ ] Replace the placeholder gradient icon with real artwork.

## CI artifact

**`road-a-tauri-app`** — contains the `.app` bundle and `.dmg` from
`src-tauri/target/release/bundle/`.

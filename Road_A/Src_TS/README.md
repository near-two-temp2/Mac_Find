# Mac_Find — Road A · TypeScript (Electron + searchfs)

Road A implementation in **TypeScript**: a macOS GUI desktop app (Electron)
whose search backend calls the kernel `searchfs(2)` syscall through a Node
**native addon** (node-gyp / C++). No index — every query is a real-time
catalog search, the same technique Windows Everything uses on NTFS.

```
Src_TS/
├── native/searchfs.cc        C++ N-API addon wrapping searchfs(2) + fsgetpath()
├── binding.gyp               node-gyp build config for the addon
├── src/
│   ├── main.ts               Electron main process (window + IPC)
│   ├── preload.ts            contextIsolation bridge (window.macFind)
│   ├── engine.ts             typed TS wrapper that loads the native addon
│   └── cli.ts                headless CLI entry (CI smoke test / scripting)
├── renderer/
│   ├── index.html            search box + result list
│   ├── styles.css            native-ish, dark-mode aware
│   └── renderer.ts           UI logic (compiled in place to renderer.js)
├── tsconfig.json             main-process build (CommonJS / Node)
├── tsconfig.renderer.json    renderer build (ES module / DOM)
├── package.json              scripts + electron-builder config
└── package-lock.json         committed lockfile (npm ci)
```

## How it works

1. The C++ addon (`native/searchfs.cc`) is a straight port of the reference
   `Open_Ref/searchfs/main.m`: it fills an `fssearchblock`, sets `SRCHFS_*`
   flags from the JS options, calls `searchfs()` over `/` and
   `/System/Volumes/Data` (the Catalina+ split), resolves each `fsid + objid`
   to a path with `fsgetpath()`, and returns a `string[]` to JS. EBUSY (catalog
   changed) is retried; EAGAIN paginates.
2. `engine.ts` loads the compiled addon via `bindings('searchfs')` and exposes
   a typed `search(term, options)`.
3. The Electron **main process** (`main.ts`) runs the search on an IPC request
   and returns results. The **renderer** debounces keystrokes and renders.

## Build

Authoritative build is **GitHub Actions on `macos-latest`** (the addon links
against macOS-only `searchfs`/`fsgetpath`). Local macOS 12 dev machines are not
required to build.

```bash
npm ci                 # install deps from the committed lockfile
npm run build:addon    # compile the C++ native addon (node-gyp)
npm run build:main     # tsc: main process -> dist/
npm run build:renderer # tsc: renderer.ts -> renderer/renderer.js
npm run pack           # electron-builder -> release/ (.app + .dmg)

# or all of the above:
npm run build
```

Run the GUI locally:

```bash
npm start              # compile + launch Electron
```

Headless CLI (used by CI as a smoke test):

```bash
node dist/cli.js --check          # prints {"engineAvailable":true|false}
node dist/cli.js -f -m 20 report  # 20 files whose name contains "report"
node dist/cli.js --help
```

## Search options

Exposed both in the GUI toolbar and on the CLI:

| GUI control      | CLI flag                | searchfs behaviour                 |
| ---------------- | ----------------------- | ---------------------------------- |
| Files only       | `-f, --files-only`      | `SRCHFS_MATCHFILES` only           |
| Dirs only        | `-d, --dirs-only`       | `SRCHFS_MATCHDIRS` only            |
| Exact            | `-e, --exact`           | drops `SRCHFS_MATCHPARTIALNAMES`   |
| Case-sensitive   | `-s, --case-sensitive`  | post-filter on basename            |
| Skip hidden      | `-i, --skip-invisibles` | `SRCHFS_SKIPINVISIBLE`             |
| (CLI only)       | `-p, --skip-packages`   | `SRCHFS_SKIPPACKAGES`              |
| Limit            | `-m, --limit <n>`       | stop after n matches (per query)   |

Matching is **substring on the filename** (kernel does case-insensitive partial
match by default); this is filename search only, not content search.

## Implemented

- C++ N-API addon calling `searchfs()` + `fsgetpath()`, dual-volume search.
- Options: files-only / dirs-only, substring vs exact, case-sensitive,
  skip-invisibles, skip-packages, result limit.
- Electron GUI: search box, debounced live search, result list (name + path),
  option toolbar, status bar with count + timing, engine-unavailable banner.
- CLI entry with `--check` engine probe for CI.
- Committed `package-lock.json` so CI uses `npm ci`.
- Own workflow `.github/workflows/build-a-ts.yml`, `runs-on: macos-latest`,
  uploads artifact **`road-a-ts-app`** (`.dmg` + unpacked `.app`).

## TODO

- **Full Disk Access**: unsigned CI builds can `searchfs` the user's data only
  after the app is granted Full Disk Access in System Settings; document/prompt
  this in-app.
- Move the (synchronous) native search off the main process onto a worker /
  async N-API `AsyncWorker` so very large result sets never block the UI.
- Stream results incrementally to the renderer instead of one batch.
- Volume picker (`-v` / `--list` equivalent) and per-volume toggles.
- `^prefix` / `suffix$` anchor modifiers like the reference CLI.
- Code-signing + notarization for a distributable, permission-friendly build.
- Reveal-in-Finder / open on double-click; keyboard navigation of results.
- App icon and DMG background.

## CI artifact

Workflow: `.github/workflows/build-a-ts.yml`
Artifact name: **`road-a-ts-app`** (contains the `.dmg` and the unpacked
`mac*/…/*.app` bundle).

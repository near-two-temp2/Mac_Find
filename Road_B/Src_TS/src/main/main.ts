// main.ts — Electron main process for the Road_B (TS) GUI.
//
// Owns the search engine: builds the index on demand and answers search queries
// from the renderer over IPC. The renderer never touches the filesystem — it
// only sends a query string and renders the hits.

import { app, BrowserWindow, ipcMain } from "electron";
import * as path from "path";
import * as os from "os";
import { buildIndex } from "../engine/indexer";
import { Searcher } from "../engine/searcher";
import { IndexReader } from "../engine/indexReader";
import {
  saveIndex,
  loadIndex,
  indexExists,
  defaultCacheDir,
  IndexMeta,
} from "../engine/store";
import { SearchOptions } from "../engine/searcher";

let win: BrowserWindow | null = null;
let searcher: Searcher | null = null;
let meta: IndexMeta | null = null;

function createWindow(): void {
  win = new BrowserWindow({
    width: 900,
    height: 640,
    minWidth: 560,
    minHeight: 360,
    title: "MacFind · Electron/TS · Road_B",
    webPreferences: {
      preload: path.join(__dirname, "..", "preload", "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });
  win.loadFile(path.join(__dirname, "..", "renderer", "index.html"));
}

// Try to load a previously-built index at startup so search works immediately.
function tryLoadExisting(): void {
  try {
    if (indexExists()) {
      const loaded = loadIndex();
      searcher = new Searcher(loaded.reader);
      meta = loaded.meta;
    }
  } catch (e) {
    // Corrupt / stale index — ignore, user can rebuild from the GUI.
    searcher = null;
    meta = null;
  }
}

function defaultRoots(): string[] {
  // Home is the sensible default scope; users can expand later. On non-mac dev
  // boxes this still works for local testing.
  return [os.homedir()];
}

// ---- IPC handlers ----------------------------------------------------------

ipcMain.handle("index:status", () => {
  return {
    ready: searcher !== null,
    meta,
    cacheDir: defaultCacheDir(),
  };
});

ipcMain.handle(
  "index:build",
  async (_evt, args: { roots?: string[]; maxEntries?: number }) => {
    const roots = args?.roots && args.roots.length > 0 ? args.roots : defaultRoots();
    const maxEntries = args?.maxEntries ?? 400_000;

    // Report coarse progress back to the renderer.
    const serialized = buildIndex({
      roots,
      maxEntries,
      onProgress: (count) => {
        if (win && !win.isDestroyed()) win.webContents.send("index:progress", count);
      },
    });
    meta = saveIndex(serialized, roots);
    // Re-open from disk to exercise the same read path the CLI uses.
    const reader = new IndexReader(serialized.blob, serialized.extTable);
    searcher = new Searcher(reader);
    return { ok: true, meta };
  }
);

ipcMain.handle(
  "search:query",
  (_evt, args: { query: string; opts?: SearchOptions }) => {
    if (!searcher) return { hits: [], error: "no index" };
    const t0 = Date.now();
    const hits = searcher.search(args.query, args.opts ?? {});
    return { hits, ms: Date.now() - t0, indexed: meta?.entryCount ?? 0 };
  }
);

// ---- App lifecycle ---------------------------------------------------------

app.whenReady().then(() => {
  tryLoadExisting();
  createWindow();
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});

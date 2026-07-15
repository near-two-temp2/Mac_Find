/*
 * main.ts — Electron main process for the Road_C hybrid GUI.
 *
 * Owns the HybridEngine (index primary + searchfs fallback) and exposes it to
 * the renderer over a small, typed IPC surface:
 *   'engine:status'  -> { hasIndex, entries, fallbackReady, fallbackError }
 *   'engine:search'  -> HybridSearchResult
 *   'engine:reindex' -> entry count
 *   'shell:reveal'   -> reveal a path in Finder
 *   'shell:open'     -> open a path with the default app
 */

import { app, BrowserWindow, ipcMain, shell } from 'electron';
import * as path from 'path';
import {
  HybridEngine,
  defaultRoots,
} from '../engine/hybridEngine';

// Compiled worker script lives alongside the engine output.
const WORKER_SCRIPT = path.join(__dirname, '..', 'engine', 'searchWorker.js');

let engine: HybridEngine | null = null;
let mainWindow: BrowserWindow | null = null;

function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 900,
    height: 640,
    minWidth: 560,
    minHeight: 400,
    title: 'MacFind · Electron/TS · Road_C',
    titleBarStyle: 'hiddenInset',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false, // preload needs require() for the bridge
    },
  });

  mainWindow.loadFile(path.join(__dirname, '..', 'renderer', 'index.html'));

  mainWindow.on('closed', () => {
    mainWindow = null;
  });
}

async function ensureEngine(): Promise<HybridEngine> {
  if (!engine) {
    engine = new HybridEngine({ workerScript: WORKER_SCRIPT });
    await engine.init();
  }
  return engine;
}

function registerIpc(): void {
  ipcMain.handle('engine:status', async () => {
    const e = await ensureEngine();
    return {
      hasIndex: e.hasIndex(),
      entries: e.indexEntryCount(),
      fallbackReady: e.fallbackReady(),
      fallbackError: e.fallbackError(),
    };
  });

  ipcMain.handle(
    'engine:search',
    async (_evt, pattern: string, opts: { limit?: number; dirsOnly?: boolean; filesOnly?: boolean }) => {
      const e = await ensureEngine();
      return e.search(pattern, opts ?? {});
    }
  );

  ipcMain.handle(
    'engine:reindex',
    async (_evt, roots?: string[], max?: number) => {
      const e = await ensureEngine();
      const targets = roots && roots.length ? roots : defaultRoots();
      return e.rebuildIndex(targets, max ?? 100000);
    }
  );

  ipcMain.handle('shell:reveal', async (_evt, filePath: string) => {
    shell.showItemInFolder(filePath);
    return true;
  });

  ipcMain.handle('shell:open', async (_evt, filePath: string) => {
    const err = await shell.openPath(filePath);
    return err === '' ? true : err;
  });
}

app.whenReady().then(() => {
  registerIpc();
  createWindow();

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit();
});

app.on('before-quit', async () => {
  if (engine) await engine.dispose();
});

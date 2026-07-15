/*
 * main.ts — Electron main process for Mac_Find Road_A (searchfs, no index).
 *
 * Creates the window, wires up an IPC "search" handler that calls the native
 * searchfs addon on a debounced request from the renderer, and streams results
 * back. The search itself is synchronous in the addon but fast (kernel catalog
 * search), so we run it inline in the main process.
 */

import { app, BrowserWindow, ipcMain } from 'electron';
import * as path from 'path';
import { search, isEngineAvailable, SearchOptions } from './engine';

let mainWindow: BrowserWindow | null = null;

function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 900,
    height: 640,
    minWidth: 560,
    minHeight: 360,
    title: 'MacFind · Electron/TS · Road_A',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

  mainWindow.loadFile(path.join(__dirname, '..', 'renderer', 'index.html'));

  mainWindow.on('closed', () => {
    mainWindow = null;
  });
}

// IPC: renderer asks main to run a search. Returns { ok, results } or
// { ok:false, error } so the renderer can show a friendly message when the
// native engine is missing (e.g. unsigned build lacking Full Disk Access).
ipcMain.handle(
  'search',
  (_event, term: string, options: SearchOptions): { ok: boolean; results?: string[]; error?: string } => {
    try {
      const results = search(term, options);
      return { ok: true, results };
    } catch (err) {
      return { ok: false, error: (err as Error).message };
    }
  },
);

// IPC: quick capability probe so the UI can warn up front.
ipcMain.handle('engine-available', (): boolean => isEngineAvailable());

app.whenReady().then(() => {
  createWindow();

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on('window-all-closed', () => {
  // For a single-window search utility, quitting on last window close is the
  // friendlier behaviour on every platform.
  app.quit();
});

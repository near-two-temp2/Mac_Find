/*
 * preload.ts — secure IPC bridge (contextIsolation on, nodeIntegration off).
 *
 * Exposes a minimal `window.macfind` API to the renderer. No Node internals or
 * ipcRenderer object leak into the page — only these typed methods.
 */

import { contextBridge, ipcRenderer } from 'electron';

export interface EngineStatus {
  hasIndex: boolean;
  entries: number;
  fallbackReady: boolean;
  fallbackError: string | null;
}

export interface SearchOptions {
  limit?: number;
  dirsOnly?: boolean;
  filesOnly?: boolean;
}

export interface SearchHit {
  path: string;
  isDir: boolean;
  score: number;
}

export interface SearchResult {
  mode: 'index' | 'searchfs' | 'none';
  results: SearchHit[];
  tookMs: number;
  note?: string;
}

const api = {
  status: (): Promise<EngineStatus> => ipcRenderer.invoke('engine:status'),
  search: (pattern: string, opts?: SearchOptions): Promise<SearchResult> =>
    ipcRenderer.invoke('engine:search', pattern, opts ?? {}),
  reindex: (roots?: string[], max?: number): Promise<number> =>
    ipcRenderer.invoke('engine:reindex', roots, max),
  reveal: (filePath: string): Promise<boolean> =>
    ipcRenderer.invoke('shell:reveal', filePath),
  open: (filePath: string): Promise<boolean | string> =>
    ipcRenderer.invoke('shell:open', filePath),
};

export type MacFindApi = typeof api;

contextBridge.exposeInMainWorld('macfind', api);

// preload.ts — safe IPC bridge exposed to the renderer via contextBridge.
//
// The renderer runs with contextIsolation on and no node integration; this is
// the only surface it can call. Each method maps 1:1 to a main-process handler.

import { contextBridge, ipcRenderer } from "electron";

export interface SearchHitDTO {
  path: string;
  score: number;
  isDir: boolean;
  start: number;
  end: number;
}

const api = {
  indexStatus: () => ipcRenderer.invoke("index:status"),
  buildIndex: (roots?: string[], maxEntries?: number) =>
    ipcRenderer.invoke("index:build", { roots, maxEntries }),
  search: (query: string, opts?: Record<string, unknown>) =>
    ipcRenderer.invoke("search:query", { query, opts }),
  onProgress: (cb: (count: number) => void) => {
    const listener = (_e: unknown, count: number) => cb(count);
    ipcRenderer.on("index:progress", listener);
    return () => ipcRenderer.removeListener("index:progress", listener);
  },
};

contextBridge.exposeInMainWorld("macfind", api);

export type MacFindAPI = typeof api;

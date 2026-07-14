/*
 * preload.ts — safe bridge between renderer and main.
 *
 * With contextIsolation on, the renderer can't touch Node/Electron directly.
 * We expose a minimal, typed `window.macFind` API over IPC.
 */

import { contextBridge, ipcRenderer } from 'electron';
import type { SearchOptions } from './engine';

export interface SearchResponse {
  ok: boolean;
  results?: string[];
  error?: string;
}

contextBridge.exposeInMainWorld('macFind', {
  search: (term: string, options: SearchOptions): Promise<SearchResponse> =>
    ipcRenderer.invoke('search', term, options),
  engineAvailable: (): Promise<boolean> => ipcRenderer.invoke('engine-available'),
});

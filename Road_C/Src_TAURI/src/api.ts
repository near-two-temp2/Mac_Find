// Thin wrappers over the Rust #[tauri::command] handlers.

import { invoke } from "@tauri-apps/api/core";
import type { EngineStatus, SearchOptions, SearchResponse } from "./types";

export function search(
  query: string,
  options: SearchOptions,
): Promise<SearchResponse> {
  return invoke<SearchResponse>("search", { query, options });
}

export function searchLive(
  query: string,
  options: SearchOptions,
): Promise<SearchResponse> {
  return invoke<SearchResponse>("search_live", { query, options });
}

export function engineStatus(): Promise<EngineStatus> {
  return invoke<EngineStatus>("engine_status");
}

export function rebuildIndex(roots?: string[]): Promise<number> {
  return invoke<number>("rebuild_index", { roots: roots ?? null });
}

export function revealInFinder(path: string): Promise<void> {
  return invoke("reveal_in_finder", { path });
}

export function openPath(path: string): Promise<void> {
  return invoke("open_path", { path });
}

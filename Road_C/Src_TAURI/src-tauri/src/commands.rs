//! `#[tauri::command]` handlers exposed to the TypeScript frontend, plus the
//! Finder-reveal helper. These are thin wrappers over `engine::Engine`.

use crate::engine::types::{EngineStatus, SearchOptions, SearchResponse};
use crate::engine::Engine;
use std::path::PathBuf;
use tauri::State;

/// Managed state: the shared hybrid engine.
pub struct AppState {
    pub engine: Engine,
}

/// Primary search. Picks index or searchfs automatically.
#[tauri::command]
pub fn search(
    query: String,
    options: Option<SearchOptions>,
    state: State<'_, AppState>,
) -> Result<SearchResponse, String> {
    let opts = options.unwrap_or_default();
    state.engine.search(&query, &opts)
}

/// Force a live searchfs search (UI "live" toggle / fallback demonstration).
#[tauri::command]
pub fn search_live(
    query: String,
    options: Option<SearchOptions>,
    state: State<'_, AppState>,
) -> Result<SearchResponse, String> {
    let opts = options.unwrap_or_default();
    state.engine.search_live(&query, &opts)
}

/// Report engine status (index ready? entry count? fallback available?).
#[tauri::command]
pub fn engine_status(state: State<'_, AppState>) -> EngineStatus {
    state.engine.status()
}

/// (Re)build the index over the default roots (or the provided ones).
/// Runs synchronously on a blocking Tauri task from the frontend's side.
#[tauri::command]
pub fn rebuild_index(
    roots: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let roots: Vec<PathBuf> = match roots {
        Some(r) if !r.is_empty() => r.into_iter().map(PathBuf::from).collect(),
        _ => Engine::default_roots(),
    };
    state.engine.rebuild(&roots)
}

/// Reveal a path in Finder (selecting the item), mirroring KatSearch's
/// "Show in Finder". Falls back to opening the enclosing folder.
#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("`open -R` exited with status {status}"))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("reveal_in_finder is only supported on macOS".into())
    }
}

/// Open a file/folder with the default handler (double-click behavior).
#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open")
            .arg(&path)
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("`open` exited with status {status}"))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("open_path is only supported on macOS".into())
    }
}

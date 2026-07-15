//! Tauri command layer for Road A (Tauri).
//!
//! The heavy lifting lives in [`searchfs`]; here we just expose it to the
//! TypeScript front end via `#[tauri::command]` and wire up the app.

pub mod searchfs;

use searchfs::{search, SearchQuery, SearchResult};

/// Run a live filename search via the macOS `searchfs()` syscall.
///
/// Called from TS as `invoke("search_files", { query })`. Runs on a blocking
/// thread pool (`async`) so the syscall's up-to-1s-per-volume time limit never
/// stalls the UI event loop.
#[tauri::command]
async fn search_files(query: SearchQuery) -> Result<SearchResult, String> {
    // searchfs() is a blocking syscall; hop onto a blocking thread so the
    // async runtime stays responsive.
    tauri::async_runtime::spawn_blocking(move || search(&query))
        .await
        .map_err(|e| format!("search task panicked: {e}"))
}

/// Report basic engine capabilities to the UI (shown in the status bar).
#[tauri::command]
fn engine_info() -> serde_json::Value {
    serde_json::json!({
        "engine": "searchfs",
        "road": "A",
        "stack": "Tauri (Rust backend + TypeScript frontend)",
        "index": false,
    })
}

/// Standard Tauri entry point. `mac-find-tauri` (bin) and mobile targets call
/// this. Kept in the lib so integration tests / other bins can reuse it.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![search_files, engine_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

//! Road_C · Tauri — macOS hybrid file-search app.
//!
//! Library crate shared by the GUI binary (`main.rs`) and the CLI smoke-test
//! binary (`cli.rs`). Exposes the engine plus the Tauri `run()` entry point.

pub mod commands;
pub mod engine;

use commands::AppState;
use engine::Engine;

/// Build and run the Tauri application. Called from `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            engine: Engine::new(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::search_live,
            commands::engine_status,
            commands::rebuild_index,
            commands::reveal_in_finder,
            commands::open_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

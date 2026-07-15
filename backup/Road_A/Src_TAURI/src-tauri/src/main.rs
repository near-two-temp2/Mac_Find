// Prevents an additional console window on Windows in release. Harmless on
// macOS (our only target), kept for Tauri convention.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mac_find_tauri_lib::run();
}

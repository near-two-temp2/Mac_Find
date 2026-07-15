// Road_B (Tauri) — GUI 桌面 app 入口。
// 关闭 Windows 上的控制台窗口（macOS 无影响，保留惯例）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    haifind_tauri_lib::run();
}

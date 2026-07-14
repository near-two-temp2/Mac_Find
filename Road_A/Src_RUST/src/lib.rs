//! Road_A (Rust) — macOS instant filename search backed by `searchfs(2)`.
//!
//! Shared crate library so both the GUI (`mac-find-gui`) and the CLI smoke-test
//! binary (`mac-find-cli`) reuse the same engine.

#[cfg(target_os = "macos")]
pub mod searchfs_sys;

pub mod engine;

pub use engine::{search, SearchHit, SearchOptions};

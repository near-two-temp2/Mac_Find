//! Shared types crossing the Rust engine ↔ TS frontend boundary.

use serde::{Deserialize, Serialize};

/// One search result. Serialized to the frontend as camelCase JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    /// Absolute path.
    pub path: String,
    /// Last path component.
    pub name: String,
    /// True when the entry is a directory.
    pub is_dir: bool,
    /// Fuzzy score (index engine only; 0 for searchfs fallback hits).
    pub score: i32,
}

impl Hit {
    pub fn from_path(path: &str, score: i32) -> Self {
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();
        let is_dir = std::path::Path::new(path).is_dir();
        Hit {
            path: path.to_string(),
            name,
            is_dir,
            score,
        }
    }

    /// Like `from_path` but takes the is_dir flag directly (avoids a stat()
    /// when the index already recorded it).
    pub fn new(path: String, name: String, is_dir: bool, score: i32) -> Self {
        Hit {
            path,
            name,
            is_dir,
            score,
        }
    }
}

/// Options accompanying a search request.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchOptions {
    pub files_only: bool,
    pub dirs_only: bool,
    pub case_sensitive: bool,
    pub skip_packages: bool,
    pub skip_invisibles: bool,
    /// Anchor match to the start of the basename (searchfs `^`).
    pub match_start: bool,
    /// Anchor match to the end of the basename (searchfs `$`).
    pub match_end: bool,
    /// Hard cap on returned hits (0 = unlimited).
    pub limit: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions {
            files_only: false,
            dirs_only: false,
            case_sensitive: false,
            skip_packages: false,
            skip_invisibles: false,
            match_start: false,
            match_end: false,
            limit: 1000,
        }
    }
}

/// Which engine produced a result set, reported back to the UI so it can
/// show the active mode (index vs. live fallback).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    /// Self-built mmap binary index + fzf scoring (primary).
    Index,
    /// searchfs(2) live catalog search (fallback).
    Searchfs,
}

/// Full response for a search command.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub engine: EngineKind,
    pub hits: Vec<Hit>,
    /// Total candidates that survived phase-1 pre-filter (for diagnostics).
    pub scanned: usize,
    /// Wall-clock time in milliseconds.
    pub elapsed_ms: u64,
}

/// Reported to the UI on startup / index (re)build so it can show status.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    /// Whether a usable index is loaded in memory.
    pub index_ready: bool,
    /// Number of entries in the loaded index.
    pub index_entries: usize,
    /// Whether the searchfs fallback is available on this machine.
    pub searchfs_available: bool,
    /// Path to the on-disk index file.
    pub index_path: String,
}

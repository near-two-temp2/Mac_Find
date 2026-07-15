//! Hybrid search engine for Road_C.
//!
//! Primary path: the self-built mmap binary index (`index.rs`) — rayon
//! parallel bitmask pre-filter + fzf scoring. When the index is missing or
//! corrupt on disk, or a search is requested before any index has been built,
//! we transparently fall back to the live `searchfs(2)` syscall
//! (`searchfs.rs`) so results are still 100% accurate, just slower.
//!
//! `Engine` is `Send + Sync` (guarded by an `RwLock`) so it can live in Tauri
//! managed state and be shared across command invocations.

pub mod fzf;
pub mod index;
pub mod searchfs;
pub mod types;

use std::path::PathBuf;
use std::sync::RwLock;
use std::time::Instant;

use index::Index;
use types::{EngineKind, EngineStatus, SearchOptions, SearchResponse};

pub struct Engine {
    /// Loaded index, if any. `None` means "no usable index → use fallback".
    index: RwLock<Option<Index>>,
    index_path: PathBuf,
}

impl Engine {
    /// Create the engine and attempt to load an existing index from the
    /// default cache location. A missing/corrupt index is not an error —
    /// the engine simply starts in fallback mode.
    pub fn new() -> Self {
        Self::with_index_path(Index::default_path())
    }

    pub fn with_index_path(index_path: PathBuf) -> Self {
        let loaded = Index::load(&index_path).ok();
        Engine {
            index: RwLock::new(loaded),
            index_path,
        }
    }

    /// Snapshot of current engine state for the UI.
    pub fn status(&self) -> EngineStatus {
        let guard = self.index.read().unwrap();
        EngineStatus {
            index_ready: guard.as_ref().map_or(false, |i| !i.is_empty()),
            index_entries: guard.as_ref().map_or(0, |i| i.len()),
            searchfs_available: searchfs::is_available(),
            index_path: self.index_path.display().to_string(),
        }
    }

    /// Run a query. Uses the index when ready, otherwise searchfs.
    pub fn search(&self, query: &str, opts: &SearchOptions) -> Result<SearchResponse, String> {
        let start = Instant::now();

        // Fast path: index loaded and non-empty.
        {
            let guard = self.index.read().unwrap();
            if let Some(idx) = guard.as_ref() {
                if !idx.is_empty() {
                    let (hits, scanned) = idx.search(query, opts);
                    return Ok(SearchResponse {
                        engine: EngineKind::Index,
                        hits,
                        scanned,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    });
                }
            }
        }

        // Fallback: live searchfs.
        let hits = searchfs::search(query, opts)?;
        let scanned = hits.len();
        Ok(SearchResponse {
            engine: EngineKind::Searchfs,
            hits,
            scanned,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Force a searchfs search regardless of index state (used by the UI's
    /// "live search" toggle and for parity testing).
    pub fn search_live(&self, query: &str, opts: &SearchOptions) -> Result<SearchResponse, String> {
        let start = Instant::now();
        let hits = searchfs::search(query, opts)?;
        let scanned = hits.len();
        Ok(SearchResponse {
            engine: EngineKind::Searchfs,
            hits,
            scanned,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// (Re)build the index over `roots`, then hot-swap it into place. Blocks
    /// until complete; callers should run this off the UI thread.
    pub fn rebuild(&self, roots: &[PathBuf]) -> Result<usize, String> {
        let count = Index::build(roots, &self.index_path).map_err(|e| e.to_string())?;
        let fresh = Index::load(&self.index_path).map_err(|e| e.to_string())?;
        *self.index.write().unwrap() = Some(fresh);
        Ok(count)
    }

    /// Default roots to index: the user's home directory and /Applications.
    /// Kept modest so a first build finishes quickly on CI and dev machines.
    pub fn default_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home));
        }
        let apps = PathBuf::from("/Applications");
        if apps.exists() {
            roots.push(apps);
        }
        if roots.is_empty() {
            roots.push(PathBuf::from("."));
        }
        roots
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

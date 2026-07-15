//! Primary engine: self-built mmap-friendly binary index with a 64-bit
//! letter bitmask pre-filter + rayon-parallel phase-1 scan + fzf phase-2
//! scoring. Modeled on Cling's `.idx` design (see `open-source-analysis.md`
//! §3.3–3.4), simplified to what Road_C needs.
//!
//! On-disk layout (little-endian):
//!   magic:   u64  = 0x4d46_4958_5f43_5230  ("MFIX_CR0")
//!   count:   u64  = number of entries
//!   bytelen: u64  = total length of the packed path blob
//!   then, per entry i:
//!       masks[i]:      u64   path letter bitmask
//!       bn_masks[i]:   u64   basename letter bitmask
//!       offsets[i]:    u32   offset into the blob
//!       lengths[i]:    u16   path byte length
//!       bn_starts[i]:  u16   basename start (offset within the path)
//!       is_dirs[i]:    u8    1 = directory
//!   finally:
//!       blob: all lowercased UTF-8 paths concatenated
//!
//! Everything is a parallel array so the hot loop touches contiguous memory.

use crate::engine::fzf;
use crate::engine::types::{Hit, SearchOptions};
use rayon::prelude::*;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use walkdir::WalkDir;

const MAGIC: u64 = 0x4d46_4958_5f43_5230; // "MFIX_CR0"

/// Compute the 64-bit letter bitmask for a lowercased byte slice.
///
/// Bits 0-25: a-z, 26-35: 0-9, 36: '.', 37: '-', 38: '_'.
#[inline]
fn mask_of(bytes: &[u8]) -> u64 {
    let mut m: u64 = 0;
    for &b in bytes {
        let bit = match b {
            b'a'..=b'z' => (b - b'a') as u32,
            b'0'..=b'9' => 26 + (b - b'0') as u32,
            b'.' => 36,
            b'-' => 37,
            b'_' => 38,
            _ => continue,
        };
        m |= 1u64 << bit;
    }
    m
}

/// An in-memory (owned) index. For simplicity we read the whole file into a
/// Vec rather than mmap'ing — the layout is still mmap-friendly and this keeps
/// the code portable and safe. Millions of entries stay comfortably in RAM.
pub struct Index {
    masks: Vec<u64>,
    bn_masks: Vec<u64>,
    offsets: Vec<u32>,
    lengths: Vec<u16>,
    bn_starts: Vec<u16>,
    is_dirs: Vec<u8>,
    blob: Vec<u8>, // packed, lowercased
    path: PathBuf,
}

impl Index {
    pub fn len(&self) -> usize {
        self.masks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.masks.is_empty()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Default index location under the user cache directory.
    pub fn default_path() -> PathBuf {
        let base = dirs_cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        base.join("com.macfind.roadc.tauri").join("index.idx")
    }

    // ---- build ------------------------------------------------------------

    /// Walk `roots` and build a fresh index, writing it to `out_path`.
    /// Returns the number of entries indexed. Follows no symlinks; silently
    /// skips unreadable directories (permission-limited CI is fine).
    pub fn build(roots: &[PathBuf], out_path: &Path) -> io::Result<usize> {
        let mut masks = Vec::new();
        let mut bn_masks = Vec::new();
        let mut offsets = Vec::new();
        let mut lengths = Vec::new();
        let mut bn_starts = Vec::new();
        let mut is_dirs = Vec::new();
        let mut blob: Vec<u8> = Vec::new();

        for root in roots {
            for entry in WalkDir::new(root)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                let path_str = match path.to_str() {
                    Some(s) => s,
                    None => continue,
                };
                if path_str.len() > u16::MAX as usize {
                    continue;
                }
                let lower = path_str.to_ascii_lowercase();
                let lower_bytes = lower.as_bytes();

                let bn_start = path_str
                    .rfind('/')
                    .map(|i| i + 1)
                    .unwrap_or(0)
                    .min(u16::MAX as usize);

                masks.push(mask_of(lower_bytes));
                bn_masks.push(mask_of(&lower_bytes[bn_start..]));
                offsets.push(blob.len() as u32);
                lengths.push(lower_bytes.len() as u16);
                bn_starts.push(bn_start as u16);
                is_dirs.push(entry.file_type().is_dir() as u8);
                blob.extend_from_slice(lower_bytes);
            }
        }

        let count = masks.len();
        let idx = Index {
            masks,
            bn_masks,
            offsets,
            lengths,
            bn_starts,
            is_dirs,
            blob,
            path: out_path.to_path_buf(),
        };
        idx.write_to(out_path)?;
        Ok(count)
    }

    fn write_to(&self, out_path: &Path) -> io::Result<()> {
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Write to a temp file then rename, so a crash mid-write can't leave a
        // truncated (corrupt) index that would poison the next launch.
        let tmp = out_path.with_extension("idx.tmp");
        {
            let mut w = io::BufWriter::new(fs::File::create(&tmp)?);
            let count = self.masks.len() as u64;
            w.write_all(&MAGIC.to_le_bytes())?;
            w.write_all(&count.to_le_bytes())?;
            w.write_all(&(self.blob.len() as u64).to_le_bytes())?;
            for i in 0..self.masks.len() {
                w.write_all(&self.masks[i].to_le_bytes())?;
                w.write_all(&self.bn_masks[i].to_le_bytes())?;
                w.write_all(&self.offsets[i].to_le_bytes())?;
                w.write_all(&self.lengths[i].to_le_bytes())?;
                w.write_all(&self.bn_starts[i].to_le_bytes())?;
                w.write_all(&[self.is_dirs[i]])?;
            }
            w.write_all(&self.blob)?;
            w.flush()?;
        }
        fs::rename(&tmp, out_path)?;
        Ok(())
    }

    // ---- load -------------------------------------------------------------

    /// Load an index from disk. Returns an error (treated as "index missing/
    /// corrupt" by the caller) on bad magic, truncation, or size mismatch.
    pub fn load(path: &Path) -> io::Result<Index> {
        let mut buf = Vec::new();
        fs::File::open(path)?.read_to_end(&mut buf)?;
        if buf.len() < 24 {
            return Err(corrupt("header too short"));
        }
        let magic = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        if magic != MAGIC {
            return Err(corrupt("bad magic"));
        }
        let count = u64::from_le_bytes(buf[8..16].try_into().unwrap()) as usize;
        let blob_len = u64::from_le_bytes(buf[16..24].try_into().unwrap()) as usize;

        // Per-entry record size: 8 + 8 + 4 + 2 + 2 + 1 = 25 bytes.
        const REC: usize = 25;
        let entries_end = 24 + count * REC;
        if buf.len() != entries_end + blob_len {
            return Err(corrupt("size mismatch"));
        }

        let mut masks = Vec::with_capacity(count);
        let mut bn_masks = Vec::with_capacity(count);
        let mut offsets = Vec::with_capacity(count);
        let mut lengths = Vec::with_capacity(count);
        let mut bn_starts = Vec::with_capacity(count);
        let mut is_dirs = Vec::with_capacity(count);

        let mut p = 24;
        for _ in 0..count {
            masks.push(u64::from_le_bytes(buf[p..p + 8].try_into().unwrap()));
            bn_masks.push(u64::from_le_bytes(buf[p + 8..p + 16].try_into().unwrap()));
            offsets.push(u32::from_le_bytes(buf[p + 16..p + 20].try_into().unwrap()));
            lengths.push(u16::from_le_bytes(buf[p + 20..p + 22].try_into().unwrap()));
            bn_starts.push(u16::from_le_bytes(buf[p + 22..p + 24].try_into().unwrap()));
            is_dirs.push(buf[p + 24]);
            p += REC;
        }
        let blob = buf[entries_end..].to_vec();

        Ok(Index {
            masks,
            bn_masks,
            offsets,
            lengths,
            bn_starts,
            is_dirs,
            blob,
            path: path.to_path_buf(),
        })
    }

    // ---- search -----------------------------------------------------------

    /// Two-phase search. Returns (hits, candidates_scanned).
    ///
    /// Phase 1 (parallel, O(n)): bitmask pre-filter + option gating.
    /// Phase 2: fzf score on the survivors' basenames, then a full-path fzf
    /// pass so queries containing `/` still match. Sort by score desc.
    pub fn search(&self, query: &str, opts: &SearchOptions) -> (Vec<Hit>, usize) {
        if query.is_empty() {
            return (Vec::new(), 0);
        }
        let q_lower = query.to_ascii_lowercase();
        let q_bytes = q_lower.as_bytes();
        let q_mask = mask_of(q_bytes);

        let scanned = AtomicUsize::new(0);

        // Phase 1 + 2 fused: iterate in parallel, keep only scored survivors.
        let mut scored: Vec<(i32, usize)> = (0..self.masks.len())
            .into_par_iter()
            .filter_map(|i| {
                // Bitmask pre-filter: if the path is missing any letter the
                // query needs, it cannot match. One u64 AND rejects ~99%.
                if self.masks[i] & q_mask != q_mask {
                    return None;
                }
                let is_dir = self.is_dirs[i] != 0;
                if opts.files_only && is_dir {
                    return None;
                }
                if opts.dirs_only && !is_dir {
                    return None;
                }

                scanned.fetch_add(1, Ordering::Relaxed);

                let off = self.offsets[i] as usize;
                let len = self.lengths[i] as usize;
                let path_bytes = &self.blob[off..off + len];
                let bn_off = self.bn_starts[i] as usize;
                let bn = &path_bytes[bn_off..];

                // Prefer basename match; fall back to full-path (needed for
                // queries with '/'). Basename hits get a boost so filename
                // matches float above deep-path incidental matches.
                let score = if self.bn_masks[i] & q_mask == q_mask {
                    fzf::score(q_bytes, bn, bn).map(|s| s + 32)
                } else {
                    None
                }
                .or_else(|| fzf::score(q_bytes, path_bytes, path_bytes))?;

                Some((score, i))
            })
            .collect();

        // Highest score first; ties broken by shorter path (more specific).
        scored.par_sort_unstable_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| self.lengths[a.1].cmp(&self.lengths[b.1]))
        });

        let limit = if opts.limit == 0 {
            scored.len()
        } else {
            opts.limit
        };

        let hits = scored
            .iter()
            .take(limit)
            .map(|&(score, i)| {
                let off = self.offsets[i] as usize;
                let len = self.lengths[i] as usize;
                // blob is lowercased; good enough for display. (A production
                // build would store original case separately — see TODO.)
                let path = String::from_utf8_lossy(&self.blob[off..off + len]).into_owned();
                let bn_off = self.bn_starts[i] as usize;
                let name = String::from_utf8_lossy(&self.blob[off + bn_off..off + len])
                    .into_owned();
                Hit::new(path, name, self.is_dirs[i] != 0, score)
            })
            .collect();

        (hits, scanned.load(Ordering::Relaxed))
    }
}

fn corrupt(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("index corrupt: {msg}"))
}

/// Minimal cache-dir resolver so we don't pull in the `dirs` crate.
fn dirs_cache_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library").join("Caches"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_load_search_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("mfidx_test_{}", std::process::id()));
        let root = tmp.join("root");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("readme.md"), b"x").unwrap();
        fs::write(root.join("sub").join("config.toml"), b"y").unwrap();

        let idx_path = tmp.join("test.idx");
        let n = Index::build(&[root.clone()], &idx_path).unwrap();
        assert!(n >= 3, "expected at least 3 entries, got {n}");

        let idx = Index::load(&idx_path).unwrap();
        let opts = SearchOptions::default();

        let (hits, _) = idx.search("readme", &opts);
        assert!(hits.iter().any(|h| h.name.contains("readme")));

        let (hits2, _) = idx.search("config", &opts);
        assert!(hits2.iter().any(|h| h.name.contains("config")));

        let (none, _) = idx.search("zzzznotthere", &opts);
        assert!(none.is_empty());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_corrupt_index() {
        let tmp = std::env::temp_dir().join(format!("mfidx_bad_{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let p = tmp.join("bad.idx");
        fs::write(&p, b"not a real index").unwrap();
        assert!(Index::load(&p).is_err());
        let _ = fs::remove_dir_all(&tmp);
    }
}

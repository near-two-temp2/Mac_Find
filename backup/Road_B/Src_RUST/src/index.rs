//! 二进制索引：建索引（遍历文件系统 → 并行数组 → 落盘）与 mmap 读取。
//!
//! 磁盘布局（小端，mmap 友好，所有区段 8 字节对齐）：
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │ Header (64 bytes)                                         │
//! │   magic:        u64  = 0x_31_42_49_46_49_41_48 ("HAIFIB1")│
//! │   version:      u32  = 1                                  │
//! │   _pad:         u32                                       │
//! │   entry_count:  u64                                       │
//! │   bytes_len:    u64  (allBytes 总字节数)                   │
//! │   off_masks:    u64  (各区段相对文件起始的偏移)            │
//! │   off_bnmasks:  u64                                       │
//! │   off_bounds:   u64                                       │
//! │   off_meta:     u64  (offset/len/bnstart/extid/isdir 打包) │
//! ├──────────────────────────────────────────────────────────┤
//! │ masks[]:     u64 × entry_count   路径字母 bitmask          │
//! │ bnMasks[]:   u64 × entry_count   basename 字母 bitmask     │
//! │ bounds[]:    u64 × entry_count   词边界位图                │
//! │ meta[]:      EntryMeta × entry_count（见下）               │
//! │ allBytes:    打包的小写 UTF-8 路径字节                     │
//! └──────────────────────────────────────────────────────────┘
//! ```
//!
//! `EntryMeta`（16 bytes，`repr(C)`）：byte_offset(u32) / byte_len(u16) /
//! bn_start(u16) / ext_id(u32) / is_dir(u8) / _pad(u8×3)。

use crate::bitmask;
use memmap2::Mmap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use walkdir::WalkDir;

pub const MAGIC: u64 = 0x0048_4149_4649_4231; // "HAIFIB1" 风格常量
pub const VERSION: u32 = 1;
pub const HEADER_LEN: usize = 64;

/// 每条目定长元数据（与磁盘布局一致）。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EntryMeta {
    pub byte_offset: u32,
    pub byte_len: u16,
    pub bn_start: u16,
    pub ext_id: u32,
    pub is_dir: u8,
    pub _pad: [u8; 3],
}

const META_SIZE: usize = std::mem::size_of::<EntryMeta>();

/// 建索引统计信息。
#[derive(Debug, Default, Clone)]
pub struct IndexStats {
    pub entry_count: u64,
    pub bytes_len: u64,
    pub file_size: u64,
}

// ── 扩展名 ID：把 basename 的扩展名映射到一个稳定 u32（FNV-1a hash） ──
/// 空扩展名（无点）映射为 0。
pub fn ext_id_of(basename_lower: &[u8]) -> u32 {
    // 取最后一个 '.' 之后的部分作为扩展名；开头的 '.'（隐藏文件）不算扩展名。
    let ext = match basename_lower.iter().rposition(|&b| b == b'.') {
        Some(0) | None => return 0,
        Some(p) => &basename_lower[p + 1..],
    };
    if ext.is_empty() {
        return 0;
    }
    let mut hash: u32 = 0x811c_9dc5;
    for &b in ext {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    // 保留 0 表示「无扩展名」，故把 0 挪到 1。
    if hash == 0 {
        1
    } else {
        hash
    }
}

/// 一条待写入的索引记录（建索引阶段的中间态）。
struct Record {
    lower: Vec<u8>, // 小写路径字节
    mask: u64,
    bn_mask: u64,
    bounds: u64,
    bn_start: u16,
    ext_id: u32,
    is_dir: bool,
}

fn make_record(path_str: &str, is_dir: bool) -> Option<Record> {
    let bytes = path_str.as_bytes();
    if bytes.len() > u16::MAX as usize {
        return None; // 超长路径跳过（罕见）
    }
    let lower: Vec<u8> = bytes.iter().map(|b| b.to_ascii_lowercase()).collect();
    let bn_start = lower
        .iter()
        .rposition(|&b| b == b'/')
        .map(|p| p + 1)
        .unwrap_or(0);
    let basename = &lower[bn_start..];
    Some(Record {
        mask: bitmask::mask_of(&lower),
        bn_mask: bitmask::mask_of(basename),
        bounds: bitmask::word_boundaries(&lower),
        bn_start: bn_start as u16,
        ext_id: ext_id_of(basename),
        is_dir,
        lower,
    })
}

/// 索引写入器：遍历若干根路径，把文件系统条目写成二进制索引。
pub struct IndexWriter {
    records: Vec<Record>,
}

impl Default for IndexWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexWriter {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// 遍历一个根路径，加入其下所有文件与目录。
    ///
    /// 使用 walkdir（不跟随符号链接），跳过无权限项。数量上限用于 CI/演示防爆。
    pub fn add_root<P: AsRef<Path>>(&mut self, root: P, max_entries: Option<usize>) {
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if let Some(cap) = max_entries {
                if self.records.len() >= cap {
                    break;
                }
            }
            let is_dir = entry.file_type().is_dir();
            if let Some(path_str) = entry.path().to_str() {
                if let Some(rec) = make_record(path_str, is_dir) {
                    self.records.push(rec);
                }
            }
        }
    }

    /// 直接加入一条路径（供测试 / 合成数据使用）。
    pub fn add_path(&mut self, path: &str, is_dir: bool) {
        if let Some(rec) = make_record(path, is_dir) {
            self.records.push(rec);
        }
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// 序列化并写入索引文件（会创建父目录）。
    pub fn write_to<P: AsRef<Path>>(&self, out: P) -> io::Result<IndexStats> {
        if let Some(parent) = out.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = File::create(&out)?;
        let mut w = BufWriter::new(file);

        let n = self.records.len() as u64;

        // 计算 allBytes 总长与各条目偏移。
        let mut byte_offsets: Vec<u32> = Vec::with_capacity(self.records.len());
        let mut running: u64 = 0;
        for r in &self.records {
            byte_offsets.push(running as u32);
            running += r.lower.len() as u64;
        }
        let bytes_len = running;

        // 各区段偏移（相对文件起始）。
        let off_masks = HEADER_LEN as u64;
        let off_bnmasks = off_masks + n * 8;
        let off_bounds = off_bnmasks + n * 8;
        let off_meta = off_bounds + n * 8;
        let off_bytes = off_meta + n * META_SIZE as u64;

        // ── Header ──
        w.write_all(&MAGIC.to_le_bytes())?;
        w.write_all(&VERSION.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?; // _pad
        w.write_all(&n.to_le_bytes())?;
        w.write_all(&bytes_len.to_le_bytes())?;
        w.write_all(&off_masks.to_le_bytes())?;
        w.write_all(&off_bnmasks.to_le_bytes())?;
        w.write_all(&off_bounds.to_le_bytes())?;
        w.write_all(&off_meta.to_le_bytes())?;
        // header 已写 8*8 = 64 字节，正好 HEADER_LEN。

        // ── masks[] ──
        for r in &self.records {
            w.write_all(&r.mask.to_le_bytes())?;
        }
        // ── bnMasks[] ──
        for r in &self.records {
            w.write_all(&r.bn_mask.to_le_bytes())?;
        }
        // ── bounds[] ──
        for r in &self.records {
            w.write_all(&r.bounds.to_le_bytes())?;
        }
        // ── meta[] ──
        for (i, r) in self.records.iter().enumerate() {
            w.write_all(&byte_offsets[i].to_le_bytes())?;
            w.write_all(&(r.lower.len() as u16).to_le_bytes())?;
            w.write_all(&r.bn_start.to_le_bytes())?;
            w.write_all(&r.ext_id.to_le_bytes())?;
            w.write_all(&[r.is_dir as u8, 0, 0, 0])?; // is_dir + _pad
        }
        // ── allBytes ──
        for r in &self.records {
            w.write_all(&r.lower)?;
        }

        w.flush()?;
        let file_size = off_bytes + bytes_len;
        Ok(IndexStats {
            entry_count: n,
            bytes_len,
            file_size,
        })
    }
}

/// mmap 索引读取器：零拷贝提供各并行数组的切片视图。
pub struct IndexReader {
    _mmap: Mmap, // 保持映射存活
    entry_count: usize,
    // 以下均为指向 mmap 内部的裸偏移，通过方法安全暴露为切片。
    off_masks: usize,
    off_bnmasks: usize,
    off_bounds: usize,
    off_meta: usize,
    off_bytes: usize,
    bytes_len: usize,
}

impl IndexReader {
    /// 打开并 mmap 一个索引文件。
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = File::open(&path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        Self::from_mmap(mmap)
    }

    fn from_mmap(mmap: Mmap) -> io::Result<Self> {
        if mmap.len() < HEADER_LEN {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "索引文件过小"));
        }
        let rd_u64 = |off: usize| -> u64 {
            u64::from_le_bytes(mmap[off..off + 8].try_into().unwrap())
        };
        let magic = rd_u64(0);
        if magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "索引 magic 不匹配（文件损坏或格式不符）",
            ));
        }
        let version = u32::from_le_bytes(mmap[8..12].try_into().unwrap());
        if version != VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("索引版本 {version} 不受支持"),
            ));
        }
        let entry_count = rd_u64(16) as usize;
        let bytes_len = rd_u64(24) as usize;
        let off_masks = rd_u64(32) as usize;
        let off_bnmasks = rd_u64(40) as usize;
        let off_bounds = rd_u64(48) as usize;
        let off_meta = rd_u64(56) as usize;
        let off_bytes = off_meta + entry_count * META_SIZE;

        // 越界校验，避免 mmap 切片 panic。
        let end = off_bytes + bytes_len;
        if end > mmap.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "索引区段越界（文件被截断）",
            ));
        }
        Ok(Self {
            _mmap: mmap,
            entry_count,
            off_masks,
            off_bnmasks,
            off_bounds,
            off_meta,
            off_bytes,
            bytes_len,
        })
    }

    pub fn entry_count(&self) -> usize {
        self.entry_count
    }

    #[inline]
    fn u64_slice(&self, off: usize) -> &[u64] {
        // SAFETY: 偏移与长度在 from_mmap 中已校验，且 u64 无对齐要求违背
        // （mmap 页对齐，各区段起点 8 字节对齐）。
        let ptr = self._mmap[off..].as_ptr() as *const u64;
        unsafe { std::slice::from_raw_parts(ptr, self.entry_count) }
    }

    #[inline]
    pub fn masks(&self) -> &[u64] {
        self.u64_slice(self.off_masks)
    }

    #[inline]
    pub fn bn_masks(&self) -> &[u64] {
        self.u64_slice(self.off_bnmasks)
    }

    #[inline]
    pub fn bounds(&self) -> &[u64] {
        self.u64_slice(self.off_bounds)
    }

    /// 读取第 i 条元数据。
    #[inline]
    pub fn meta(&self, i: usize) -> EntryMeta {
        let base = self.off_meta + i * META_SIZE;
        let m = &self._mmap;
        EntryMeta {
            byte_offset: u32::from_le_bytes(m[base..base + 4].try_into().unwrap()),
            byte_len: u16::from_le_bytes(m[base + 4..base + 6].try_into().unwrap()),
            bn_start: u16::from_le_bytes(m[base + 6..base + 8].try_into().unwrap()),
            ext_id: u32::from_le_bytes(m[base + 8..base + 12].try_into().unwrap()),
            is_dir: m[base + 12],
            _pad: [0; 3],
        }
    }

    /// 第 i 条的小写路径字节切片（零拷贝）。
    #[inline]
    pub fn path_bytes(&self, i: usize) -> &[u8] {
        let meta = self.meta(i);
        let start = self.off_bytes + meta.byte_offset as usize;
        let end = start + meta.byte_len as usize;
        &self._mmap[start..end]
    }

    /// 整个 allBytes 区（供批量并行访问）。
    #[inline]
    pub fn all_bytes(&self) -> &[u8] {
        &self._mmap[self.off_bytes..self.off_bytes + self.bytes_len]
    }

    #[inline]
    pub fn bytes_base(&self) -> usize {
        self.off_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_roundtrip() {
        let mut w = IndexWriter::new();
        w.add_path("/Users/me/src/main.rs", false);
        w.add_path("/Users/me/src", true);
        w.add_path("/Applications/Xcode.app", true);

        let tmp =
            std::env::temp_dir().join(format!("haifind_test_idx_{}.idx", std::process::id()));
        let stats = w.write_to(&tmp).unwrap();
        assert_eq!(stats.entry_count, 3);

        let r = IndexReader::open(&tmp).unwrap();
        assert_eq!(r.entry_count(), 3);
        assert_eq!(r.path_bytes(0), b"/users/me/src/main.rs");
        let m0 = r.meta(0);
        assert_eq!(m0.is_dir, 0);
        // basename "main.rs" 从索引 14 开始
        assert_eq!(m0.bn_start, 14);
        // 目录项
        assert_eq!(r.meta(1).is_dir, 1);

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn ext_id_basics() {
        assert_eq!(ext_id_of(b"noext"), 0);
        assert_eq!(ext_id_of(b".hidden"), 0); // 隐藏文件无扩展名
        assert_ne!(ext_id_of(b"main.rs"), 0);
        assert_eq!(ext_id_of(b"a.rs"), ext_id_of(b"b.rs")); // 同扩展名同 ID
        assert_ne!(ext_id_of(b"a.rs"), ext_id_of(b"a.swift"));
    }
}

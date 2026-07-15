//! 自建二进制索引 —— mmap 友好的并行数组格式（参考 Cling .idx，见 §3.3）。
//!
//! 磁盘布局（小端）：
//! ```text
//! ┌─ Header (32 bytes) ───────────────────────────────────────┐
//! │   magic:        u64  = 0x48414946_43524331 ("HAIFCRC1")   │
//! │   version:      u32  = 1                                   │
//! │   entry_count:  u32                                        │
//! │   bytes_len:    u64  (allBytes 总字节数)                    │
//! │   reserved:     u64  = 0                                   │
//! ├─ Parallel arrays（每条 entry i，紧密排列，无对齐填充）────────┤
//! │   masks[i]:        u64  （小写全路径的字母 bitmask）         │
//! │   bn_masks[i]:     u64  （basename 的字母 bitmask）          │
//! │   boundaries[i]:   u64  （path 前 64 字节词边界位图）        │
//! │   offsets[i]:      u32  （path 在 allBytes 中的起始偏移）    │
//! │   lengths[i]:      u32  （path 字节长度）                    │
//! │   bn_starts[i]:    u32  （basename 在 path 中的起始偏移）    │
//! │   flags[i]:        u32  （bit0 = is_dir）                    │
//! ├─ Bulk data ───────────────────────────────────────────────┤
//! │   all_bytes: 打包的小写 UTF-8 路径字节（无分隔符）           │
//! └───────────────────────────────────────────────────────────┘
//! ```
//!
//! 写：全量遍历文件系统 → 内存收集 → 一次性落盘。
//! 读：`mmap` 整个文件，零拷贝按偏移取路径字节。

use crate::bitmask::{mask_of, word_boundaries};
use memmap2::Mmap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

pub const MAGIC: u64 = 0x4841_4946_4352_4331; // "HAIFCRC1"
pub const VERSION: u32 = 1;
pub const HEADER_LEN: usize = 32;

/// 每条 entry 定长记录的字节数：3×u64 + 4×u32 = 24 + 16 = 40。
const RECORD_LEN: usize = 40;

/// 一条待写入的索引项（建索引阶段的内存表示）。
struct RawEntry {
    mask: u64,
    bn_mask: u64,
    boundaries: u64,
    offset: u32,
    length: u32,
    bn_start: u32,
    flags: u32,
}

/// 索引写入器：收集条目，最后 [`finish`](IndexWriter::finish) 落盘。
pub struct IndexWriter {
    entries: Vec<RawEntry>,
    all_bytes: Vec<u8>,
}

/// 建索引统计。
#[derive(Clone, Copy, Debug, Default)]
pub struct IndexStats {
    pub entries: usize,
    pub bytes_len: usize,
}

impl IndexWriter {
    pub fn new() -> Self {
        IndexWriter {
            entries: Vec::new(),
            all_bytes: Vec::new(),
        }
    }

    /// 追加一个绝对路径条目。`is_dir` 标记是否为目录。
    pub fn add(&mut self, path: &str, is_dir: bool) {
        let lower: Vec<u8> = path.bytes().map(|b| b.to_ascii_lowercase()).collect();
        if lower.len() > u32::MAX as usize {
            return;
        }
        let offset = self.all_bytes.len() as u32;
        let length = lower.len() as u32;

        // basename 起点：最后一个 '/' 之后。
        let bn_start = lower
            .iter()
            .rposition(|&b| b == b'/')
            .map(|p| p + 1)
            .unwrap_or(0);
        let bn_mask = mask_of(&lower[bn_start..]);
        let mask = mask_of(&lower);
        let boundaries = word_boundaries(&lower);

        self.all_bytes.extend_from_slice(&lower);
        self.entries.push(RawEntry {
            mask,
            bn_mask,
            boundaries,
            offset,
            length,
            bn_start: bn_start as u32,
            flags: if is_dir { 1 } else { 0 },
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 序列化到 `path`（会创建父目录）。返回统计。
    pub fn finish(self, path: &Path) -> io::Result<IndexStats> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // 先写临时文件再原子 rename，避免读端看到半截索引。
        let tmp = path.with_extension("idx.tmp");
        let file = File::create(&tmp)?;
        let mut w = BufWriter::new(file);

        let entry_count = self.entries.len() as u32;
        let bytes_len = self.all_bytes.len() as u64;

        // Header
        w.write_all(&MAGIC.to_le_bytes())?;
        w.write_all(&VERSION.to_le_bytes())?;
        w.write_all(&entry_count.to_le_bytes())?;
        w.write_all(&bytes_len.to_le_bytes())?;
        w.write_all(&0u64.to_le_bytes())?; // reserved

        // 并行数组（逐条按固定顺序写字段，读端按 RECORD_LEN 定位）。
        for e in &self.entries {
            w.write_all(&e.mask.to_le_bytes())?;
            w.write_all(&e.bn_mask.to_le_bytes())?;
            w.write_all(&e.boundaries.to_le_bytes())?;
            w.write_all(&e.offset.to_le_bytes())?;
            w.write_all(&e.length.to_le_bytes())?;
            w.write_all(&e.bn_start.to_le_bytes())?;
            w.write_all(&e.flags.to_le_bytes())?;
        }

        // Bulk 路径字节。
        w.write_all(&self.all_bytes)?;
        w.flush()?;
        drop(w);

        std::fs::rename(&tmp, path)?;

        Ok(IndexStats {
            entries: entry_count as usize,
            bytes_len: bytes_len as usize,
        })
    }
}

impl Default for IndexWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// 只读、mmap 映射的一条 entry 视图（零拷贝）。
#[derive(Clone, Copy)]
pub struct Entry<'a> {
    pub mask: u64,
    pub bn_mask: u64,
    pub boundaries: u64,
    pub bn_start: usize,
    pub is_dir: bool,
    /// 小写全路径字节切片。
    pub path: &'a [u8],
}

impl<'a> Entry<'a> {
    /// basename 的小写字节切片。
    #[inline]
    pub fn basename(&self) -> &'a [u8] {
        &self.path[self.bn_start.min(self.path.len())..]
    }
}

/// mmap 索引读取器。
pub struct IndexReader {
    _mmap: Mmap,
    entry_count: usize,
    /// 指向记录区起点（Header 之后）。
    records_ptr: *const u8,
    /// 指向 bulk 字节区起点。
    bytes_ptr: *const u8,
    bytes_len: usize,
    // 保证 mmap 存活期间指针有效；只读、单线程构造后共享 &self 只读访问。
    _marker: std::marker::PhantomData<()>,
}

// IndexReader 在构造后只做只读访问；mmap 区域在 reader 存活期间不变。
unsafe impl Send for IndexReader {}
unsafe impl Sync for IndexReader {}

impl IndexReader {
    /// 打开并校验索引文件。头部魔数/版本不符或长度不足时返回错误（视为「索引损坏」）。
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        let data = &mmap[..];

        if data.len() < HEADER_LEN {
            return Err(corrupt("index too small for header"));
        }
        let magic = u64::from_le_bytes(data[0..8].try_into().unwrap());
        if magic != MAGIC {
            return Err(corrupt("bad magic"));
        }
        let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
        if version != VERSION {
            return Err(corrupt("unsupported index version"));
        }
        let entry_count = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
        let bytes_len = u64::from_le_bytes(data[16..24].try_into().unwrap()) as usize;

        let records_len = entry_count
            .checked_mul(RECORD_LEN)
            .ok_or_else(|| corrupt("entry count overflow"))?;
        let expected = HEADER_LEN
            .checked_add(records_len)
            .and_then(|v| v.checked_add(bytes_len))
            .ok_or_else(|| corrupt("size overflow"))?;
        if data.len() < expected {
            return Err(corrupt("truncated index (records/bytes shorter than header claims)"));
        }

        let base = data.as_ptr();
        let records_ptr = unsafe { base.add(HEADER_LEN) };
        let bytes_ptr = unsafe { base.add(HEADER_LEN + records_len) };

        Ok(IndexReader {
            _mmap: mmap,
            entry_count,
            records_ptr,
            bytes_ptr,
            bytes_len,
            _marker: std::marker::PhantomData,
        })
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entry_count
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entry_count == 0
    }

    /// 按索引取第 `i` 条 entry（零拷贝）。`i` 必须 < len()。
    #[inline]
    pub fn entry(&self, i: usize) -> Entry<'_> {
        debug_assert!(i < self.entry_count);
        let rec = unsafe { self.records_ptr.add(i * RECORD_LEN) };
        let mask = read_u64(rec, 0);
        let bn_mask = read_u64(rec, 8);
        let boundaries = read_u64(rec, 16);
        let offset = read_u32(rec, 24) as usize;
        let length = read_u32(rec, 28) as usize;
        let bn_start = read_u32(rec, 32) as usize;
        let flags = read_u32(rec, 36);

        let off = offset.min(self.bytes_len);
        let end = (offset + length).min(self.bytes_len);
        let path =
            unsafe { std::slice::from_raw_parts(self.bytes_ptr.add(off), end.saturating_sub(off)) };

        Entry {
            mask,
            bn_mask,
            boundaries,
            bn_start,
            is_dir: flags & 1 != 0,
            path,
        }
    }
}

#[inline]
fn read_u64(rec: *const u8, off: usize) -> u64 {
    let mut buf = [0u8; 8];
    unsafe {
        std::ptr::copy_nonoverlapping(rec.add(off), buf.as_mut_ptr(), 8);
    }
    u64::from_le_bytes(buf)
}

#[inline]
fn read_u32(rec: *const u8, off: usize) -> u32 {
    let mut buf = [0u8; 4];
    unsafe {
        std::ptr::copy_nonoverlapping(rec.add(off), buf.as_mut_ptr(), 4);
    }
    u32::from_le_bytes(buf)
}

fn corrupt(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("corrupt index: {msg}"))
}

/// 默认索引文件位置：`~/Library/Caches/com.haifind.c-rust/index.idx`
///
/// 与 Road_B、Cling 思路一致：落在用户缓存目录，磁盘紧张时可安全删除
/// （删除后混合引擎自动降级到 searchfs() 实时兜底）。
pub fn default_index_path() -> PathBuf {
    let base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("com.haifind.c-rust").join("index.idx")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrip() {
        let dir = std::env::temp_dir().join(format!("haifind-c-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.idx");

        let mut w = IndexWriter::new();
        w.add("/Users/x/Src/main.rs", false);
        w.add("/Users/x/Src", true);
        let stats = w.finish(&path).unwrap();
        assert_eq!(stats.entries, 2);

        let r = IndexReader::open(&path).unwrap();
        assert_eq!(r.len(), 2);
        let e0 = r.entry(0);
        assert_eq!(e0.path, b"/users/x/src/main.rs");
        assert_eq!(e0.basename(), b"main.rs");
        assert!(!e0.is_dir);
        let e1 = r.entry(1);
        assert!(e1.is_dir);
        assert_eq!(e1.basename(), b"src");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_bad_magic() {
        let dir = std::env::temp_dir().join(format!("haifind-c-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.idx");
        std::fs::write(&path, b"not a real index file at all!!!!").unwrap();
        assert!(IndexReader::open(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}

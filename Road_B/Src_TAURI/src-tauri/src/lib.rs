//! Road_B (Tauri) — 自建二进制索引 + fzf 模糊搜索
//!
//! Rust 后端 = 搜索引擎（见 [`engine`]）：
//!   - 建索引：遍历文件系统 → 写 mmap 友好的二进制索引（并行数组：
//!     小写路径字节、64-bit 字母 bitmask、basename bitmask、词边界、扩展名 ID）。
//!   - 查索引：mmap 加载 → Phase 1 rayon 并行 bitmask/扩展名预过滤 →
//!     Phase 2 对存活候选做 fzf 评分排序。
//!
//! 通过 `#[tauri::command]` 把「建索引 / 搜索 / 状态」暴露给 TypeScript 前端。
//! 前端（`../src/`）用 `@tauri-apps/api` 的 `invoke` 调用这些命令并渲染结果列表。

pub mod engine;

use engine::{default_index_path, search as run_search, IndexReader, IndexWriter, KindFilter, Match, SearchOptions};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

/// 全局应用状态：当前已 mmap 打开的索引（若有）及其来源路径。
///
/// 用 `Mutex` 保护；搜索是只读的，但重建索引会替换 reader，故需要可变访问。
#[derive(Default)]
pub struct AppState {
    inner: Mutex<Option<LoadedIndex>>,
}

struct LoadedIndex {
    reader: IndexReader,
    path: PathBuf,
}

/// 建索引结果，回传给前端展示。
#[derive(Serialize)]
pub struct IndexResult {
    pub entry_count: u64,
    pub bytes_len: u64,
    pub file_size: u64,
    pub index_path: String,
    pub elapsed_ms: u128,
}

/// 索引状态，供前端在启动时探测「是否已有可用索引」。
#[derive(Serialize)]
pub struct IndexStatus {
    pub loaded: bool,
    pub entry_count: u64,
    pub index_path: String,
}

/// 前端 search 命令的返回体：结果列表 + 命中数 + 耗时。
#[derive(Serialize)]
pub struct SearchResponse {
    pub results: Vec<Match>,
    pub total: usize,
    pub elapsed_ms: u128,
}

fn resolve_index_path(custom: &Option<String>) -> PathBuf {
    match custom {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => default_index_path(),
    }
}

/// 命令：建立索引。
///
/// - `roots`：要索引的根路径列表；为空则默认索引用户主目录（$HOME）。
/// - `max_entries`：条目上限（防止全盘遍历过久 / 演示防爆）；0 或 null 视为无上限。
/// - `out_path`：索引落盘位置；null 用默认缓存路径。
///
/// 成功后会立即把新索引 mmap 加载进 [`AppState`]，供后续 `search` 命令使用。
#[tauri::command]
fn build_index(
    state: tauri::State<'_, AppState>,
    roots: Vec<String>,
    max_entries: Option<usize>,
    out_path: Option<String>,
) -> Result<IndexResult, String> {
    let out = resolve_index_path(&out_path);
    let cap = max_entries.filter(|&n| n > 0);

    let roots: Vec<String> = if roots.is_empty() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        vec![home.to_string_lossy().into_owned()]
    } else {
        roots
    };

    let t0 = Instant::now();
    let mut writer = IndexWriter::new();
    for root in &roots {
        writer.add_root(root, cap);
    }
    let stats = writer
        .write_to(&out)
        .map_err(|e| format!("写入索引失败：{e}"))?;

    // 立即加载新索引，替换旧的。
    let reader = IndexReader::open(&out).map_err(|e| format!("加载索引失败：{e}"))?;
    {
        let mut guard = state.inner.lock().map_err(|_| "状态锁中毒".to_string())?;
        *guard = Some(LoadedIndex {
            reader,
            path: out.clone(),
        });
    }

    Ok(IndexResult {
        entry_count: stats.entry_count,
        bytes_len: stats.bytes_len,
        file_size: stats.file_size,
        index_path: out.to_string_lossy().into_owned(),
        elapsed_ms: t0.elapsed().as_millis(),
    })
}

/// 命令：加载已有索引（不重建）。启动时或用户指向自定义 .idx 文件时调用。
#[tauri::command]
fn load_index(
    state: tauri::State<'_, AppState>,
    index_path: Option<String>,
) -> Result<IndexStatus, String> {
    let path = resolve_index_path(&index_path);
    let reader = IndexReader::open(&path)
        .map_err(|e| format!("无法打开索引 {}：{e}", path.display()))?;
    let entry_count = reader.entry_count() as u64;
    {
        let mut guard = state.inner.lock().map_err(|_| "状态锁中毒".to_string())?;
        *guard = Some(LoadedIndex {
            reader,
            path: path.clone(),
        });
    }
    Ok(IndexStatus {
        loaded: true,
        entry_count,
        index_path: path.to_string_lossy().into_owned(),
    })
}

/// 命令：返回当前索引状态（是否已加载、条目数、路径）。
#[tauri::command]
fn index_status(state: tauri::State<'_, AppState>) -> Result<IndexStatus, String> {
    let guard = state.inner.lock().map_err(|_| "状态锁中毒".to_string())?;
    Ok(match guard.as_ref() {
        Some(idx) => IndexStatus {
            loaded: true,
            entry_count: idx.reader.entry_count() as u64,
            index_path: idx.path.to_string_lossy().into_owned(),
        },
        None => IndexStatus {
            loaded: false,
            entry_count: 0,
            index_path: default_index_path().to_string_lossy().into_owned(),
        },
    })
}

/// 命令：在已加载索引中搜索。
///
/// - `query`：查询串（空串返回默认视图）。
/// - `kind`：`"all"` / `"files"` / `"dirs"`。
/// - `limit`：结果上限（默认 200）。
///
/// 若尚无索引，返回明确错误，前端提示用户先「建立索引」。
#[tauri::command]
fn search(
    state: tauri::State<'_, AppState>,
    query: String,
    kind: Option<String>,
    limit: Option<usize>,
) -> Result<SearchResponse, String> {
    let guard = state.inner.lock().map_err(|_| "状态锁中毒".to_string())?;
    let idx = guard
        .as_ref()
        .ok_or_else(|| "尚无索引，请先点击「建立索引」".to_string())?;

    let opts = SearchOptions {
        limit: limit.unwrap_or(200),
        kind: kind.as_deref().map(KindFilter::parse).unwrap_or(KindFilter::All),
        use_ext_filter: true,
    };

    let t0 = Instant::now();
    let results = run_search(&idx.reader, &query, &opts);
    let total = results.len();
    Ok(SearchResponse {
        results,
        total,
        elapsed_ms: t0.elapsed().as_millis(),
    })
}

/// Tauri 应用入口。`main.rs` 与移动端入口都调用它。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            build_index,
            load_index,
            index_status,
            search
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}

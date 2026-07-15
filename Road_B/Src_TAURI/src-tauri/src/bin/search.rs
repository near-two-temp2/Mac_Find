//! CLI 入口：`haifind-tauri-search`，供 CI 冒烟测试与脚本调用。
//!
//! 复用与 Tauri GUI 完全相同的搜索引擎（`haifind_tauri_lib::engine`），
//! 因此这条 CLI 通过就等于证明后端引擎能 端到端 建索引 + 查索引。
//!
//! 用法：
//! ```text
//! haifind-tauri-search [QUERY] [--root PATH]... [--out PATH] [--max N]
//!                      [--limit N] [--files | --dirs] [--self-test]
//! ```
//! - QUERY：查询串；缺省为空（默认视图）。
//! - --root：要索引的根路径，可多次；缺省为 $HOME。
//! - --out：索引输出路径；缺省为 ~/Library/Caches/com.haifind.b-tauri/index.idx。
//! - --max：条目上限（CI 上防止全盘遍历过久）。
//! - --limit：结果上限（默认 50）。
//! - --files / --dirs：仅文件 / 仅目录。
//! - --self-test：用合成数据自检引擎（不触碰真实文件系统），CI 必过。

use haifind_tauri_lib::engine::{
    default_index_path, search, IndexReader, IndexWriter, KindFilter, SearchOptions,
};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

fn print_help() {
    println!(
        "haifind-tauri-search [QUERY] [--root PATH]... [--out PATH] [--max N] \
         [--limit N] [--files|--dirs] [--self-test]\n\
         Road_B (Tauri) 搜索引擎 CLI：建二进制索引 + 两阶段 fzf 搜索。"
    );
}

/// 用合成数据自检：建一个内存/临时索引并搜索，验证引擎端到端可用。
/// CI 环境即使没有 Full Disk Access 也必定通过。
fn self_test() -> ExitCode {
    let sample = [
        ("/Users/me/project/main.rs", false),
        ("/Users/me/project/README.md", false),
        ("/Users/me/project/src/main_helper.rs", false),
        ("/Users/me/Documents/notes.txt", false),
        ("/Applications/Xcode.app", true),
    ];
    let mut w = IndexWriter::new();
    for (p, d) in sample {
        w.add_path(p, d);
    }
    let tmp = std::env::temp_dir().join(format!("haifind_tauri_selftest_{}.idx", std::process::id()));
    let stats = match w.write_to(&tmp) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("自检失败：写索引 {e}");
            return ExitCode::FAILURE;
        }
    };
    let reader = match IndexReader::open(&tmp) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("自检失败：读索引 {e}");
            std::fs::remove_file(&tmp).ok();
            return ExitCode::FAILURE;
        }
    };
    let res = search(&reader, "main.rs", &SearchOptions::default());
    std::fs::remove_file(&tmp).ok();

    println!("自检：索引 {} 条目，查询 \"main.rs\" 命中 {} 条", stats.entry_count, res.len());
    if res.is_empty() || !res[0].path.ends_with("main.rs") {
        eprintln!("自检失败：期望首条命中 basename 为 main.rs");
        return ExitCode::FAILURE;
    }
    println!("自检通过 ✓  首条：{} (score={})", res[0].path, res[0].score);
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut query: Option<String> = None;
    let mut roots: Vec<String> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut max: Option<usize> = None;
    let mut limit: usize = 50;
    let mut kind = KindFilter::All;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "--self-test" => return self_test(),
            "--root" => {
                i += 1;
                match args.get(i) {
                    Some(p) => roots.push(p.clone()),
                    None => {
                        eprintln!("--root 需要一个路径参数");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--out" => {
                i += 1;
                match args.get(i) {
                    Some(p) => out = Some(PathBuf::from(p)),
                    None => {
                        eprintln!("--out 需要一个路径参数");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--max" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse::<usize>().ok()) {
                    Some(n) => max = Some(n),
                    None => {
                        eprintln!("--max 需要一个正整数");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--limit" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse::<usize>().ok()) {
                    Some(n) => limit = n,
                    None => {
                        eprintln!("--limit 需要一个正整数");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--files" => kind = KindFilter::FilesOnly,
            "--dirs" => kind = KindFilter::DirsOnly,
            other if !other.starts_with("--") && query.is_none() => {
                query = Some(other.to_string());
            }
            other => {
                eprintln!("未知参数：{other}");
                print_help();
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    if roots.is_empty() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        roots.push(home.to_string_lossy().into_owned());
    }
    let out = out.unwrap_or_else(default_index_path);

    // ── 建索引 ──
    let t0 = Instant::now();
    let mut writer = IndexWriter::new();
    for root in &roots {
        eprintln!("索引根路径：{root}");
        writer.add_root(root, max);
    }
    let stats = match writer.write_to(&out) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("写入索引失败：{e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "索引完成：{} 条目 · {} 字节 · 耗时 {:.2?} · {}",
        stats.entry_count,
        stats.file_size,
        t0.elapsed(),
        out.display()
    );

    // ── 查索引 ──
    let reader = match IndexReader::open(&out) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("加载索引失败：{e}");
            return ExitCode::FAILURE;
        }
    };
    let opts = SearchOptions {
        limit,
        kind,
        use_ext_filter: true,
    };
    let q = query.unwrap_or_default();
    let ts = Instant::now();
    let results = search(&reader, &q, &opts);
    eprintln!(
        "查询 \"{}\" 命中 {} 条 · 耗时 {:.2?}",
        q,
        results.len(),
        ts.elapsed()
    );
    for m in &results {
        let tag = if m.is_dir { "d" } else { "f" };
        println!("[{tag}] {:>6}  {}", m.score, m.path);
    }
    ExitCode::SUCCESS
}

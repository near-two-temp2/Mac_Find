//! Road_C (Rust) CLI —— 混合引擎命令行入口，供脚本与 CI 冒烟测试。
//!
//! 子命令：
//!   - `index [--root DIR]... [--out FILE]`   建索引（默认根=主目录，默认输出=缓存目录）
//!   - `search [--out/--index FILE] [--files-only|--dirs-only] [--limit N] QUERY`
//!                                            混合查询（有索引走索引，否则 searchfs 兜底）
//!   - `doctor`                               打印引擎能力自检（索引状态 / searchfs 是否可用）
//!
//! 无参数时打印用法并以退出码 0 结束（CI 冒烟只需能跑起来）。

use haifind_c::engine::{build_index, Backend, HybridEngine, SearchOptions};
use haifind_c::index::default_index_path;
use haifind_c::searchfs;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        return ExitCode::SUCCESS;
    }

    match args[0].as_str() {
        "index" => cmd_index(&args[1..]),
        "search" => cmd_search(&args[1..]),
        "doctor" => cmd_doctor(),
        "-h" | "--help" | "help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("未知子命令：{other}\n");
            print_usage();
            ExitCode::FAILURE
        }
    }
}

fn cmd_index(args: &[String]) -> ExitCode {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                if let Some(v) = args.get(i + 1) {
                    roots.push(PathBuf::from(v));
                    i += 1;
                }
            }
            "--out" | "--index" => {
                if let Some(v) = args.get(i + 1) {
                    out = Some(PathBuf::from(v));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if roots.is_empty() {
        roots = haifind_c::engine::default_roots();
    }
    let out = out.unwrap_or_else(default_index_path);

    eprintln!("建索引：根={roots:?} → {}", out.display());
    match build_index(&roots, &out, false) {
        Ok(stats) => {
            println!(
                "OK  entries={} bytes={} path={}",
                stats.entries,
                stats.bytes_len,
                out.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("建索引失败：{e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_search(args: &[String]) -> ExitCode {
    let mut index_path: Option<PathBuf> = None;
    let mut opts = SearchOptions::default();
    let mut query: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--index" | "--out" => {
                if let Some(v) = args.get(i + 1) {
                    index_path = Some(PathBuf::from(v));
                    i += 1;
                }
            }
            "--limit" => {
                if let Some(v) = args.get(i + 1) {
                    opts.limit = v.parse().unwrap_or(opts.limit);
                    i += 1;
                }
            }
            "--files-only" => opts.files_only = true,
            "--dirs-only" => opts.dirs_only = true,
            other => {
                if !other.starts_with("--") && query.is_none() {
                    query = Some(other.to_string());
                }
            }
        }
        i += 1;
    }

    let query = match query {
        Some(q) => q,
        None => {
            eprintln!("search 需要一个查询词。");
            return ExitCode::FAILURE;
        }
    };
    opts.query = query;

    let engine = match index_path {
        Some(p) => HybridEngine::with_index_path(p),
        None => HybridEngine::new(),
    };

    let res = engine.search(&opts);
    let backend = match res.backend {
        Backend::Index => "index",
        Backend::SearchfsFallback => "searchfs-fallback",
        Backend::Unavailable => "unavailable",
    };
    eprintln!("后端={backend}  命中={}", res.matches.len());
    for m in &res.matches {
        let kind = if m.is_dir { "d" } else { "f" };
        println!("{}\t{}\t{}", m.score, kind, m.path);
    }
    ExitCode::SUCCESS
}

fn cmd_doctor() -> ExitCode {
    let idx_path = default_index_path();
    let engine = HybridEngine::new();
    println!("Mac Hai Find — Road_C (Rust) 引擎自检");
    println!("  索引路径      : {}", idx_path.display());
    println!("  索引已加载    : {}", engine.has_index());
    println!("  索引条目数    : {}", engine.index_len());
    println!("  searchfs 可用 : {}", searchfs::available());
    let primary = if engine.has_index() {
        "索引主路径"
    } else if searchfs::available() {
        "searchfs() 实时兜底"
    } else {
        "无可用后端（非 macOS 或无 searchfs 卷）"
    };
    println!("  当前主用      : {primary}");
    ExitCode::SUCCESS
}

fn print_usage() {
    eprintln!(
        "\
haifind-c-cli — Road_C (Rust) 混合引擎 CLI

用法:
  haifind-c-cli index  [--root DIR]... [--out FILE]
  haifind-c-cli search [--index FILE] [--files-only|--dirs-only] [--limit N] QUERY
  haifind-c-cli doctor

说明:
  混合引擎：有索引走「索引 + fzf」主路径，索引缺失/损坏时降级到 searchfs() 实时兜底。
  index 默认遍历主目录、写入 ~/Library/Caches/com.haifind.c-rust/index.idx。
  GUI 版见 `haifind-c-gui`。"
    );
}

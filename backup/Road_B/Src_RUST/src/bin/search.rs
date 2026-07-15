//! CLI 入口：查索引。供 CI 冒烟测试与脚本调用。
//!
//! 用法：
//! ```text
//! haifind-search QUERY [--index PATH] [--limit N] [--files|--dirs]
//! ```

use haifind::search::KindFilter;
use haifind::{default_index_path, search, IndexReader, SearchOptions};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut query: Option<String> = None;
    let mut index_path: Option<PathBuf> = None;
    let mut opts = SearchOptions::default();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--index" => {
                i += 1;
                match args.get(i) {
                    Some(p) => index_path = Some(PathBuf::from(p)),
                    None => {
                        eprintln!("--index 需要一个路径参数");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--limit" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse::<usize>().ok()) {
                    Some(n) => opts.limit = n,
                    None => {
                        eprintln!("--limit 需要一个正整数");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--files" => opts.kind = KindFilter::FilesOnly,
            "--dirs" => opts.kind = KindFilter::DirsOnly,
            "-h" | "--help" => {
                println!(
                    "haifind-search QUERY [--index PATH] [--limit N] [--files|--dirs]\n\
                     在二进制索引中模糊搜索（Road_B / Rust）。"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                if query.is_none() {
                    query = Some(other.to_string());
                } else {
                    // 多余的裸参数拼进查询，支持含空格的多 token 查询。
                    let q = query.take().unwrap();
                    query = Some(format!("{q} {other}"));
                }
            }
        }
        i += 1;
    }

    let query = match query {
        Some(q) => q,
        None => {
            eprintln!("用法：haifind-search QUERY [--index PATH] [--limit N] [--files|--dirs]");
            return ExitCode::FAILURE;
        }
    };
    let index_path = index_path.unwrap_or_else(default_index_path);

    let reader = match IndexReader::open(&index_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("打开索引失败（{}）：{e}", index_path.display());
            eprintln!("提示：先运行 haifind-index 建立索引。");
            return ExitCode::FAILURE;
        }
    };

    let t0 = Instant::now();
    let results = search(&reader, &query, &opts);
    let dt = t0.elapsed();

    for m in &results {
        let tag = if m.is_dir { "d" } else { "f" };
        println!("[{tag}] {:>6}  {}", m.score, m.path);
    }
    eprintln!(
        "共 {} 条结果（索引 {} 条目）· 耗时 {:.2?}",
        results.len(),
        reader.entry_count(),
        dt
    );
    ExitCode::SUCCESS
}

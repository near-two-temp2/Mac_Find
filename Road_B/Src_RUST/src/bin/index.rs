//! CLI 入口：建索引。供 CI 冒烟测试与脚本调用。
//!
//! 用法：
//! ```text
//! haifind-index [ROOT ...] [--out PATH] [--max N]
//! ```
//! - ROOT：要索引的根路径，可多个；缺省为当前用户主目录（$HOME）。
//! - --out：索引输出路径；缺省为 ~/Library/Caches/com.haifind.b-rust/index.idx。
//! - --max：条目上限（防止 CI 上全盘遍历过久）；缺省无上限。

use haifind::{default_index_path, IndexWriter};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut roots: Vec<String> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut max: Option<usize> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--out 需要一个路径参数");
                    return ExitCode::FAILURE;
                }
                out = Some(PathBuf::from(&args[i]));
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
            "-h" | "--help" => {
                println!(
                    "haifind-index [ROOT ...] [--out PATH] [--max N]\n\
                     建立二进制文件索引（Road_B / Rust）。"
                );
                return ExitCode::SUCCESS;
            }
            other => roots.push(other.to_string()),
        }
        i += 1;
    }

    if roots.is_empty() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        roots.push(home.to_string_lossy().into_owned());
    }
    let out = out.unwrap_or_else(default_index_path);

    let t0 = Instant::now();
    let mut writer = IndexWriter::new();
    for root in &roots {
        eprintln!("索引根路径：{root}");
        writer.add_root(root, max);
    }

    match writer.write_to(&out) {
        Ok(stats) => {
            let dt = t0.elapsed();
            println!(
                "索引完成：{} 条目 · {} 字节路径数据 · 文件 {} 字节 · 耗时 {:.2?}",
                stats.entry_count, stats.bytes_len, stats.file_size, dt
            );
            println!("索引写入：{}", out.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("写入索引失败：{e}");
            ExitCode::FAILURE
        }
    }
}

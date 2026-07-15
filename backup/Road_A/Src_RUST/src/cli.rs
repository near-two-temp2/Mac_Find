//! CLI entry point for Road_A (Rust). Primarily a CI smoke test and scripting
//! hook for the `searchfs`-backed engine; the real deliverable is the GUI.
//!
//! Usage:
//!   mac-find-cli [flags] <search_term>
//!
//! Flags:
//!   -d, --dirs-only        Match directories only
//!   -f, --files-only       Match files only
//!   -e, --exact-match      Exact filename match (not substring)
//!   -s, --case-sensitive   Case-sensitive matching
//!   -m, --limit <N>        Stop after N matches (0 = unlimited)
//!       --self-test        Run internal sanity checks and exit 0
//!   -h, --help             Print help

use mac_find_a_rust::engine::SearchOptions;
use mac_find_a_rust::search;
use std::process::ExitCode;

fn print_help() {
    println!(
        "mac-find-cli — Road_A (Rust) searchfs() filename search\n\
         \n\
         usage: mac-find-cli [-dfesm] [--self-test] <search_term>\n\
         \n\
             -d, --dirs-only        Match directories only\n\
             -f, --files-only       Match files only\n\
             -e, --exact-match      Exact filename match (not substring)\n\
             -s, --case-sensitive   Case-sensitive matching\n\
             -m, --limit <N>        Stop after N matches (0 = unlimited)\n\
                 --self-test        Run internal sanity checks and exit\n\
             -h, --help             Print this help\n"
    );
}

/// Minimal self-test used by CI: ensures the binary links against the syscall
/// symbols and the engine runs end-to-end without panicking. We do NOT assert
/// on result counts, because the CI runner's Full Disk Access / volume layout
/// is not guaranteed — a clean run (even with zero hits) is success.
fn self_test() -> ExitCode {
    let opts = SearchOptions {
        query: "Applications".to_string(),
        limit: 5,
        ..Default::default()
    };
    let hits = search(&opts);
    println!("[self-test] engine ran, {} hit(s):", hits.len());
    for h in hits.iter().take(5) {
        println!("  {}{}", h.path, if h.is_dir { "/" } else { "" });
    }
    println!("[self-test] OK");
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let mut opts = SearchOptions::default();
    let mut term: Option<String> = None;

    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "--self-test" => return self_test(),
            "-d" | "--dirs-only" => opts.dirs_only = true,
            "-f" | "--files-only" => opts.files_only = true,
            "-e" | "--exact-match" => opts.exact_match = true,
            "-s" | "--case-sensitive" => opts.case_sensitive = true,
            "-m" | "--limit" => {
                if let Some(n) = args.next() {
                    opts.limit = n.parse().unwrap_or(opts.limit);
                } else {
                    eprintln!("error: --limit requires a number");
                    return ExitCode::from(2);
                }
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown flag '{}'", other);
                print_help();
                return ExitCode::from(2);
            }
            other => term = Some(other.to_string()),
        }
    }

    if opts.dirs_only && opts.files_only {
        eprintln!("error: --dirs-only and --files-only are mutually exclusive");
        return ExitCode::from(2);
    }

    let term = match term {
        Some(t) if !t.is_empty() => t,
        _ => {
            print_help();
            return ExitCode::from(2);
        }
    };

    opts.query = term;
    let hits = search(&opts);
    for h in &hits {
        println!("{}{}", h.path, if h.is_dir { "/" } else { "" });
    }
    eprintln!("[mac-find-cli] {} match(es)", hits.len());
    ExitCode::SUCCESS
}

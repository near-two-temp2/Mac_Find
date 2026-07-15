//! Headless CLI for CI smoke-testing the searchfs engine without a GUI.
//!
//! Usage:
//!   mac-find-cli --help
//!   mac-find-cli --self-test
//!   mac-find-cli [--files-only|--dirs-only] [--exact] [--case-sensitive]
//!               [--limit N] <term>
//!
//! Zero hits is not a failure: the CI runner may lack Full Disk Access.

use mac_find_tauri_lib::searchfs::{search, self_test, SearchQuery};

fn print_help() {
    println!(
        "mac-find-cli — Road A (Tauri) searchfs engine, headless\n\
         \n\
         USAGE:\n\
           mac-find-cli --self-test\n\
           mac-find-cli [OPTIONS] <TERM>\n\
         \n\
         OPTIONS:\n\
           --files-only        Match files only\n\
           --dirs-only         Match directories only\n\
           --exact             Whole-name exact match\n\
           --case-sensitive    Case-sensitive substring match\n\
           --limit N           Stop after N hits (0 = unlimited, default 50)\n\
           --self-test         Run the engine self-test and exit\n\
           --help, -h          Show this help"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    if args.iter().any(|a| a == "--self-test") {
        match self_test() {
            Ok(msg) => {
                println!("{msg}");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("self-test FAILED: {e}");
                std::process::exit(1);
            }
        }
    }

    let mut q = SearchQuery {
        limit: 50,
        ..Default::default()
    };
    let mut term: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--files-only" => q.files_only = true,
            "--dirs-only" => q.dirs_only = true,
            "--exact" => q.exact_match = true,
            "--case-sensitive" => q.case_sensitive = true,
            "--limit" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    q.limit = v.parse().unwrap_or(50);
                }
            }
            other if !other.starts_with("--") => term = Some(other.to_string()),
            other => {
                eprintln!("unknown flag: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    q.term = match term {
        Some(t) => t,
        None => {
            eprintln!("error: missing search term");
            print_help();
            std::process::exit(2);
        }
    };

    let res = search(&q);
    for hit in &res.hits {
        let kind = if hit.is_dir { "d" } else { "f" };
        println!("{kind}  {}", hit.path);
    }
    eprintln!(
        "--- {} hit(s), volumes={:?}, truncated={}, notes={:?}",
        res.hits.len(),
        res.volumes_searched,
        res.truncated,
        res.notes
    );
}

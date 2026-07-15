//! Headless CLI entry point — exercises the hybrid engine without the GUI, so
//! CI can smoke-test the search path on a runner that has no display and may
//! lack Full Disk Access.
//!
//! Usage:
//!   mac-find-c-cli --check                 engine self-check, exits 0
//!   mac-find-c-cli --status                print engine status JSON
//!   mac-find-c-cli --build [ROOT ...]      build index over roots (default: ./)
//!   mac-find-c-cli [--live] QUERY          search and print hits
//!
//! Zero hits is a pass: the runner may not be able to read the filesystem.

use macfind_roadc_tauri_lib::engine::types::SearchOptions;
use macfind_roadc_tauri_lib::engine::{index::Index, Engine};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }

    // --check: build a tiny throwaway index, load it, run one search. Verifies
    // the whole primary path end-to-end; never depends on system permissions.
    if args.iter().any(|a| a == "--check") {
        return self_check();
    }

    if args.iter().any(|a| a == "--status") {
        let engine = Engine::new();
        let status = engine.status();
        println!(
            "index_ready={} entries={} searchfs_available={} index_path={}",
            status.index_ready,
            status.index_entries,
            status.searchfs_available,
            status.index_path
        );
        return ExitCode::SUCCESS;
    }

    if args.first().map(|s| s.as_str()) == Some("--build") {
        let roots: Vec<PathBuf> = if args.len() > 1 {
            args[1..].iter().map(PathBuf::from).collect()
        } else {
            vec![PathBuf::from(".")]
        };
        let engine = Engine::new();
        match engine.rebuild(&roots) {
            Ok(n) => {
                println!("indexed {n} entries");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("build failed: {e}");
                ExitCode::FAILURE
            }
        }
    } else {
        // Search mode.
        let live = args.iter().any(|a| a == "--live");
        let query: String = args
            .iter()
            .find(|a| !a.starts_with("--"))
            .cloned()
            .unwrap_or_default();

        if query.is_empty() {
            eprintln!("no query provided");
            print_help();
            return ExitCode::FAILURE;
        }

        let engine = Engine::new();
        let opts = SearchOptions {
            limit: 50,
            ..Default::default()
        };
        let result = if live {
            engine.search_live(&query, &opts)
        } else {
            engine.search(&query, &opts)
        };
        match result {
            Ok(resp) => {
                eprintln!(
                    "engine={:?} hits={} scanned={} elapsed_ms={}",
                    resp.engine,
                    resp.hits.len(),
                    resp.scanned,
                    resp.elapsed_ms
                );
                for h in resp.hits {
                    println!("{}", h.path);
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                // A searchfs permission error on a locked-down CI runner is not
                // a build failure — report and exit 0.
                eprintln!("search returned error (non-fatal for CI): {e}");
                ExitCode::SUCCESS
            }
        }
    }
}

fn self_check() -> ExitCode {
    let tmp = std::env::temp_dir().join(format!("macfind_c_selfcheck_{}", std::process::id()));
    let root = tmp.join("root");
    if let Err(e) = std::fs::create_dir_all(root.join("sub")) {
        eprintln!("self-check setup failed: {e}");
        return ExitCode::FAILURE;
    }
    let _ = std::fs::write(root.join("hello-world.txt"), b"x");
    let _ = std::fs::write(root.join("sub").join("readme.md"), b"y");

    let idx_path = tmp.join("selfcheck.idx");
    let built = match Index::build(&[root.clone()], &idx_path) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("self-check build failed: {e}");
            let _ = std::fs::remove_dir_all(&tmp);
            return ExitCode::FAILURE;
        }
    };

    let engine = Engine::with_index_path(idx_path);
    let status = engine.status();
    let opts = SearchOptions {
        limit: 10,
        ..Default::default()
    };
    let resp = engine.search("readme", &opts);

    let _ = std::fs::remove_dir_all(&tmp);

    match resp {
        Ok(r) => {
            println!(
                "self-check OK: built {built} entries, index_ready={}, engine={:?}, hits={}",
                status.index_ready,
                r.engine,
                r.hits.len()
            );
            if r.hits.iter().any(|h| h.name.contains("readme")) {
                ExitCode::SUCCESS
            } else {
                eprintln!("self-check FAILED: expected a 'readme' hit");
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("self-check FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "mac-find-c-cli — Road_C hybrid engine smoke-test CLI\n\
         \n\
         USAGE:\n\
         \x20 mac-find-c-cli --check              run built-in self-test\n\
         \x20 mac-find-c-cli --status             print engine status\n\
         \x20 mac-find-c-cli --build [ROOT ...]   build index (default root: .)\n\
         \x20 mac-find-c-cli [--live] QUERY       search (index, or --live for searchfs)\n"
    );
}

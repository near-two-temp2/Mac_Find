import Foundation
import HybridEngine

/// Headless command-line surface for CI smoke tests and scripting.
enum CLI {

    static func run(_ args: [String]) -> Int32 {
        switch args.first {
        case "index":  return runIndex(Array(args.dropFirst()))
        case "search": return runSearch(Array(args.dropFirst()))
        case "--self-test": return runSelfTest()
        case "--help", "-h", nil: printUsage(); return 0
        default:
            FileHandle.standardError.write(Data("unknown command: \(args[0])\n".utf8))
            printUsage()
            return 2
        }
    }

    // MARK: index

    private static func runIndex(_ args: [String]) -> Int32 {
        var roots: [String] = []
        var includeHidden = false
        var i = 0
        while i < args.count {
            switch args[i] {
            case "--root": i += 1; if i < args.count { roots.append(args[i]) }
            case "--hidden": includeHidden = true
            default: break
            }
            i += 1
        }
        if roots.isEmpty { roots = [NSHomeDirectory()] }

        let engine = HybridEngine(roots: roots)
        var opts = IndexBuilder.Options()
        opts.includeHidden = includeHidden
        do {
            let start = Date()
            let n = try engine.buildIndex(options: opts)
            let dt = Date().timeIntervalSince(start)
            print("indexed \(n) entries from \(roots.joined(separator: ", ")) in \(String(format: "%.2f", dt))s")
            print("index: \(engine.indexPath)")
            return 0
        } catch {
            FileHandle.standardError.write(Data("index build failed: \(error)\n".utf8))
            return 1
        }
    }

    // MARK: search

    private static func runSearch(_ args: [String]) -> Int32 {
        var term: String? = nil
        var opts = SearchOptions()
        var forceFallback = false
        var i = 0
        while i < args.count {
            let a = args[i]
            switch a {
            case "--files-only": opts.filesOnly = true
            case "--dirs-only":  opts.dirsOnly = true
            case "--fallback":   forceFallback = true
            case "--limit": i += 1; if i < args.count, let n = Int(args[i]) { opts.limit = n }
            default: if !a.hasPrefix("--") && term == nil { term = a }
            }
            i += 1
        }
        guard let term else {
            FileHandle.standardError.write(Data("search: missing <term>\n".utf8))
            return 2
        }

        let engine = HybridEngine()
        let start = Date()
        let (hits, backend) = engine.search(term, options: opts, forceFallback: forceFallback)
        let dt = Date().timeIntervalSince(start) * 1000
        FileHandle.standardError.write(Data("[\(backend.rawValue)] \(hits.count) hits in \(String(format: "%.1f", dt))ms\n".utf8))
        for h in hits { print(h.path) }
        return 0
    }

    // MARK: self-test (CI gate)

    /// Build a small index in a temp dir with known files, then assert that both
    /// the index engine and the fzf ordering behave. Exercises the whole pure
    /// pipeline without a window server or root privileges.
    private static func runSelfTest() -> Int32 {
        let fm = FileManager.default
        let tmp = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("machaifind-c-selftest-\(UUID().uuidString)", isDirectory: true)
        defer { try? fm.removeItem(at: tmp) }

        do {
            try fm.createDirectory(at: tmp, withIntermediateDirectories: true)
            let files = [
                "AppManager.swift", "app_notes.txt", "README.md",
                "deep/nested/report_final.pdf", "deep/nested/teamwork.txt",
            ]
            for rel in files {
                let url = tmp.appendingPathComponent(rel)
                try fm.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
                try Data("x".utf8).write(to: url)
            }

            let idx = tmp.appendingPathComponent("test.idx").path
            let builder = IndexBuilder()
            let n = try builder.build(roots: [tmp.path], to: idx)
            guard n >= files.count else { fail("expected ≥\(files.count) entries, got \(n)"); return 1 }

            let searcher = try IndexSearcher(path: idx)

            // 1. basename fuzzy match finds the right file.
            let swiftHits = searcher.search("appman")
            guard swiftHits.contains(where: { $0.path.hasSuffix("AppManager.swift") }) else {
                fail("'appman' did not find AppManager.swift"); return 1
            }

            // 2. boundary bonus ranks AppManager above teamwork for query "am".
            let am = searcher.search("am")
            let appIdx = am.firstIndex { $0.path.hasSuffix("AppManager.swift") }
            let teamIdx = am.firstIndex { $0.path.hasSuffix("teamwork.txt") }
            if let a = appIdx, let t = teamIdx, a > t {
                fail("boundary scoring wrong: teamwork ranked above AppManager"); return 1
            }

            // 3. extension constraint filters by type.
            let pdfs = searcher.search(".pdf")
            guard pdfs.allSatisfy({ $0.path.hasSuffix(".pdf") }), !pdfs.isEmpty else {
                fail("'.pdf' extension constraint failed"); return 1
            }

            // 4. path query matches across separators.
            let nested = searcher.search("nested/report")
            guard nested.contains(where: { $0.path.hasSuffix("report_final.pdf") }) else {
                fail("path query 'nested/report' failed"); return 1
            }

            print("SELF-TEST OK — \(n) entries, all assertions passed")
            print("searchfs fallback available: \(SearchFSFallback.available)")
            return 0
        } catch {
            fail("exception: \(error)")
            return 1
        }
    }

    private static func fail(_ msg: String) {
        FileHandle.standardError.write(Data("SELF-TEST FAILED: \(msg)\n".utf8))
    }

    private static func printUsage() {
        let usage = """
        machaifind-c — Road_C hybrid file search (index + searchfs fallback)

        USAGE:
          machaifind-c [gui]                 launch the SwiftUI GUI (default)
          machaifind-c index [--root PATH]…  build the binary index (default root: $HOME)
          machaifind-c search <term> [opts]  query the engine
          machaifind-c --self-test           run the CI self-test

        SEARCH OPTIONS:
          --files-only | --dirs-only   restrict result kind
          --limit N                    cap results (default 500)
          --fallback                   force the searchfs() engine (skip index)
        """
        print(usage)
    }
}

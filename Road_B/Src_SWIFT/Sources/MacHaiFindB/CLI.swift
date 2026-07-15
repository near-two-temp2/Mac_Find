import Foundation
import SearchEngine

/// Command-line front end. Kept intentionally tiny and dependency-free so CI can
/// exercise the full build → index → search pipeline without a window server.
enum CLI {

    /// Default index location under the user's caches dir.
    static func defaultIndexURL() -> URL {
        let base = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        return base
            .appendingPathComponent("com.machaifind.roadb", isDirectory: true)
            .appendingPathComponent("index.idx")
    }

    static func printUsage() {
        let usage = """
        MacHaiFind (Road B — self-built index + fzf) — Swift

        USAGE:
          machaifind-b index  [--root PATH] [--out PATH] [--hidden]
          machaifind-b search <query> [--index PATH] [--limit N] [--files|--dirs] [--ext EXT]
          machaifind-b gui                 launch the SwiftUI GUI (default)

        Default index path: \(defaultIndexURL().path)
        """
        print(usage)
    }

    // MARK: index

    static func runIndex(_ args: [String]) -> Int32 {
        var root = FileManager.default.homeDirectoryForCurrentUser
        var out = defaultIndexURL()
        var hidden = false

        var i = 0
        while i < args.count {
            switch args[i] {
            case "--root": i += 1; if i < args.count { root = URL(fileURLWithPath: args[i]) }
            case "--out": i += 1; if i < args.count { out = URL(fileURLWithPath: args[i]) }
            case "--hidden": hidden = true
            default:
                FileHandle.standardError.write(Data("index: unknown arg \(args[i])\n".utf8))
                return 2
            }
            i += 1
        }

        let start = Date()
        let builder = IndexBuilder()
        do {
            let count = try builder.build(
                root: root,
                outputURL: out,
                includeHidden: hidden
            ) { p in
                if p.scanned % 65536 == 0 {
                    FileHandle.standardError.write(Data("  scanned \(p.scanned)…\n".utf8))
                }
            }
            let dt = Date().timeIntervalSince(start)
            print("indexed \(count) entries from \(root.path)")
            print("wrote \(out.path) in \(String(format: "%.2f", dt))s")
            return 0
        } catch {
            FileHandle.standardError.write(Data("index failed: \(error)\n".utf8))
            return 1
        }
    }

    // MARK: search

    static func runSearch(_ args: [String]) -> Int32 {
        var query: String?
        var indexURL = defaultIndexURL()
        var opts = SearchOptions()

        var i = 0
        while i < args.count {
            let a = args[i]
            switch a {
            case "--index": i += 1; if i < args.count { indexURL = URL(fileURLWithPath: args[i]) }
            case "--limit": i += 1; if i < args.count { opts.maxResults = Int(args[i]) ?? opts.maxResults }
            case "--files": opts.filesOnly = true
            case "--dirs": opts.dirsOnly = true
            case "--ext": i += 1; if i < args.count { opts.ext = args[i] }
            default:
                if a.hasPrefix("--") {
                    FileHandle.standardError.write(Data("search: unknown arg \(a)\n".utf8))
                    return 2
                }
                query = (query == nil) ? a : query! + " " + a
            }
            i += 1
        }

        guard let q = query else {
            FileHandle.standardError.write(Data("search: missing query\n".utf8))
            return 2
        }

        do {
            let engine = try SearchEngine(indexURL: indexURL)
            let start = Date()
            let results = engine.search(q, options: opts)
            let dt = Date().timeIntervalSince(start)
            for r in results {
                print("\(r.score)\t\(r.isDir ? "d" : "f")\t\(r.path)")
            }
            FileHandle.standardError.write(
                Data("\(results.count) hits over \(engine.index.count) entries in \(String(format: "%.1f", dt * 1000))ms\n".utf8)
            )
            return 0
        } catch {
            FileHandle.standardError.write(Data("search failed: \(error)\n".utf8))
            return 1
        }
    }
}

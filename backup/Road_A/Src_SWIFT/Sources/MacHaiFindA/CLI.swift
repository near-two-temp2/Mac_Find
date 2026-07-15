import Foundation
import SearchFSKit

/// Headless command-line runner. Used both as a real CLI and as the CI smoke
/// test (see `--self-test`), so the build can be exercised on a runner with no
/// window server.
enum CLI {
    static func run(_ args: [String]) -> Int32 {
        var options = SearchOptions()
        var term: String? = nil
        var volumes: [String] = []
        var selfTest = false

        var i = 0
        while i < args.count {
            let a = args[i]
            switch a {
            case "--files-only":     options.scope = .filesOnly
            case "--dirs-only":      options.scope = .dirsOnly
            case "--exact":          options.substring = false
            case "--case-sensitive": options.caseSensitive = true
            case "--self-test":      selfTest = true
            case "--limit":
                i += 1
                if i < args.count, let n = Int(args[i]) { options.limit = n }
            case "--volume":
                i += 1
                if i < args.count { volumes.append(args[i]) }
            case "-h", "--help":
                printUsage()
                return 0
            default:
                if a.hasPrefix("-") {
                    FileHandle.standardError.write("Unknown flag: \(a)\n".data(using: .utf8)!)
                    printUsage()
                    return 64 // EX_USAGE
                }
                term = a
            }
            i += 1
        }

        if selfTest {
            return runSelfTest()
        }

        guard let term = term, !term.isEmpty else {
            printUsage()
            return 64
        }

        let vols = volumes.isEmpty ? nil : volumes
        let results = SearchEngine.search(term: term, options: options, volumes: vols)
        for r in results {
            print(r.path)
        }
        return 0
    }

    /// CI smoke test: verify the searchfs pipeline is wired end-to-end without
    /// asserting on specific paths (CI filesystems vary). Success = the volume
    /// probe works and a common term returns without crashing.
    private static func runSelfTest() -> Int32 {
        let vols = SearchEngine.defaultVolumes()
        FileHandle.standardError.write("volumes: \(vols)\n".data(using: .utf8)!)
        for v in vols {
            let ok = SearchEngine.volumeSupportsSearchFS(v)
            FileHandle.standardError.write("  \(v) supports searchfs: \(ok)\n".data(using: .utf8)!)
        }

        // "usr" exists on essentially every macOS volume tree; we only require
        // that the call returns without trapping. Zero results is still a pass
        // (sandboxed/ephemeral CI volumes may legitimately return nothing).
        var opts = SearchOptions()
        opts.limit = 5
        let results = SearchEngine.search(term: "usr", options: opts)
        FileHandle.standardError.write("self-test results for 'usr' (<=5): \(results.count)\n".data(using: .utf8)!)
        for r in results { FileHandle.standardError.write("  \(r.path)\n".data(using: .utf8)!) }

        FileHandle.standardError.write("self-test OK\n".data(using: .utf8)!)
        return 0
    }

    private static func printUsage() {
        let usage = """
        MacHaiFindA — Road_A / Swift: searchfs() 无索引实时文件搜索

        用法:
          MacHaiFindA                         启动 SwiftUI GUI
          MacHaiFindA <term> [选项]           命令行搜索 (CI 冒烟用)

        选项:
          --files-only         只匹配文件
          --dirs-only          只匹配目录
          --exact              精确文件名匹配 (默认子串匹配)
          --case-sensitive     大小写敏感 (默认不敏感)
          --limit N            结果上限 (默认 1000, 0 = 无限)
          --volume PATH        指定卷 (可多次; 默认 / 和 /System/Volumes/Data)
          --self-test          运行 CI 自检并退出
          -h, --help           显示帮助

        """
        FileHandle.standardError.write(usage.data(using: .utf8)!)
    }
}

import Foundation
import CSearchFS

/// Search options exposed to the UI / CLI.
public struct SearchOptions: Sendable {
    /// What kind of filesystem objects to match.
    public enum Scope: Sendable {
        case filesAndDirs
        case filesOnly
        case dirsOnly
    }

    public var scope: Scope
    /// Substring match (true) vs. exact filename match (false).
    public var substring: Bool
    /// Case-sensitive final filtering. searchfs() itself is case-insensitive at the
    /// kernel level, so case sensitivity is enforced by us on the basename.
    public var caseSensitive: Bool
    /// Stop after this many results (0 = unlimited).
    public var limit: Int

    public init(scope: Scope = .filesAndDirs,
                substring: Bool = true,
                caseSensitive: Bool = false,
                limit: Int = 1000) {
        self.scope = scope
        self.substring = substring
        self.caseSensitive = caseSensitive
        self.limit = limit
    }
}

/// A single search hit.
public struct SearchResult: Identifiable, Hashable, Sendable {
    public let path: String
    public let name: String
    public var id: String { path }

    init(path: String) {
        self.path = path
        self.name = (path as NSString).lastPathComponent
    }
}

/// Wraps the C searchfs() driver: dual-volume scan, EBUSY retry (in C),
/// path reconstruction via fsgetpath (in C), plus higher-level filtering.
public enum SearchEngine {

    /// Returns the mount points to scan. Modern macOS (Catalina+) splits the
    /// system into a read-only `/` and a writable `/System/Volumes/Data`, so we
    /// search both by default to see the whole filesystem.
    public static func defaultVolumes() -> [String] {
        var vols = ["/"]
        if csfs_data_volume_available() != 0 {
            vols.append("/System/Volumes/Data")
        }
        return vols
    }

    /// Whether the given volume supports catalog search.
    public static func volumeSupportsSearchFS(_ path: String) -> Bool {
        path.withCString { csfs_volume_supports_searchfs($0) != 0 }
    }

    /// Box passed through the C callback's opaque context pointer.
    private final class Collector {
        let term: String
        let options: SearchOptions
        var results: [SearchResult] = []
        let shouldStop: () -> Bool

        init(term: String, options: SearchOptions, shouldStop: @escaping () -> Bool) {
            self.term = term
            self.options = options
            self.shouldStop = shouldStop
        }

        /// Higher-level filter applied on top of the kernel's match. The kernel
        /// already did a case-insensitive substring match on the *name* attribute,
        /// but fsgetpath returns the full path, so we re-check against the basename
        /// to honour case sensitivity / exact / substring precisely.
        func accept(_ path: String) -> Bool {
            let name = (path as NSString).lastPathComponent
            let hay = options.caseSensitive ? name : name.lowercased()
            let needle = options.caseSensitive ? term : term.lowercased()
            if options.substring {
                return hay.contains(needle)
            } else {
                return hay == needle
            }
        }
    }

    /// Run a synchronous search across the given volumes. `shouldStop` lets a
    /// caller cancel a search in flight (checked once per delivered result).
    public static func search(term: String,
                              options: SearchOptions,
                              volumes: [String]? = nil,
                              shouldStop: @escaping () -> Bool = { false }) -> [SearchResult] {
        let trimmed = term.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return [] }

        let vols = volumes ?? defaultVolumes()
        let collector = Collector(term: trimmed, options: options, shouldStop: shouldStop)

        // The C `enum { CSFS_* }` constants import into Swift as plain Int32/Int,
        // so combine them as integers and cast to the flags type.
        var rawFlags: Int32 = 0
        switch options.scope {
        case .filesAndDirs:
            rawFlags |= Int32(CSFS_MATCH_FILES) | Int32(CSFS_MATCH_DIRS)
        case .filesOnly:
            rawFlags |= Int32(CSFS_MATCH_FILES)
        case .dirsOnly:
            rawFlags |= Int32(CSFS_MATCH_DIRS)
        }
        if options.substring {
            rawFlags |= Int32(CSFS_PARTIAL)
        }
        let flags = csfs_options_t(UInt32(bitPattern: rawFlags))

        // The C callback: forwards each path back into the Collector, applies
        // the fine-grained filter, and enforces the limit / cancellation.
        let cb: csfs_result_cb = { cPath, ctx in
            guard let cPath = cPath, let ctx = ctx else { return 1 }
            let box = Unmanaged<Collector>.fromOpaque(ctx).takeUnretainedValue()
            if box.shouldStop() { return 0 }
            let path = String(cString: cPath)
            if box.accept(path) {
                box.results.append(SearchResult(path: path))
                if box.options.limit > 0 && box.results.count >= box.options.limit {
                    return 0 // stop
                }
            }
            return 1 // continue
        }

        let ctx = Unmanaged.passUnretained(collector).toOpaque()

        for vol in vols {
            if options.limit > 0 && collector.results.count >= options.limit { break }
            if collector.shouldStop() { break }
            _ = vol.withCString { vPtr in
                trimmed.withCString { tPtr in
                    csfs_search(vPtr, tPtr, flags, cb, ctx)
                }
            }
        }

        return collector.results
    }
}

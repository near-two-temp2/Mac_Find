import Foundation

/// The flagship hybrid search coordinator (Road_C).
///
/// Search priority:
///   1. mmap binary index (primary) — fast, parallel, fuzzy.
///   2. searchfs() catalog scan (fallback) — used when the index is
///      missing/corrupt, or explicitly forced.
///
/// Index lifecycle:
///   - `defaultIndexPath` lives under Caches so it survives across launches.
///   - `buildIndex` scans configured roots and (re)writes the index atomically.
///   - an optional `FSEventsWatcher` debounces changes and marks the index stale
///     so the owner can schedule a rebuild.
public final class HybridEngine {

    public enum Backend: String, Sendable {
        case index
        case searchfs
        case none
    }

    public struct Status: Sendable {
        public var backend: Backend
        public var entryCount: Int
        public var indexPath: String
        public var stale: Bool
    }

    private let lock = NSLock()
    private var searcher: IndexSearcher?
    private var _stale = false
    private var watcher: FSEventsWatcher?

    public let indexPath: String

    /// Roots the initial scan walks. Defaults to every *local* volume — the home
    /// dir plus each mounted apfs/hfs volume under `/Volumes` — so full-disk
    /// coverage finds e.g. both `/Users/.../temp_test` and
    /// `/Volumes/MacDisk/.../temp_test`. Network/FUSE volumes are filtered out
    /// here and again pruned during the walk (device boundary + denylist).
    public let roots: [String]

    public init(indexPath: String? = nil, roots: [String]? = nil) {
        self.indexPath = indexPath ?? Self.defaultIndexPath()
        self.roots = roots ?? Self.defaultRoots()
        _ = try? loadIndex()
    }

    /// Local-only root for a full-disk scan. A single `/` covers everything:
    ///   - the system + data volume (the walk follows macOS's firmlink so
    ///     `/Users/...` is reached), and
    ///   - every additional *local* volume under `/Volumes` (the walk crosses
    ///     into apfs/hfs mounts but the guard prunes network/FUSE ones).
    /// This is why both `/Users/oracle/temp_test` and
    /// `/Volumes/MacDisk/Users/Shared/temp_test` end up in one index.
    static func defaultRoots() -> [String] {
        ["/"]
    }

    /// Standard cache location for the index, mirroring Cling's convention.
    public static func defaultIndexPath() -> String {
        let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        let dir = caches.appendingPathComponent("com.machaifind.c", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("home.idx").path
    }

    // MARK: - Index management

    /// (Re)load the mmap index from disk. Silently leaves `searcher == nil` if
    /// the file is missing/corrupt so queries fall back to searchfs.
    @discardableResult
    public func loadIndex() throws -> Bool {
        lock.lock(); defer { lock.unlock() }
        do {
            searcher = try IndexSearcher(path: indexPath)
            _stale = false
            return true
        } catch IndexError.missing {
            searcher = nil
            return false
        } catch {
            searcher = nil
            throw error
        }
    }

    /// Build (or rebuild) the index by scanning `roots`, then hot-swap it in.
    /// Returns the number of entries indexed.
    @discardableResult
    public func buildIndex(options: IndexBuilder.Options = IndexBuilder.Options(),
                           progress: ((Int) -> Void)? = nil) throws -> Int {
        let builder = IndexBuilder()
        let count = try builder.build(roots: roots, to: indexPath, options: options)
        progress?(count)
        try loadIndex()
        return count
    }

    public var isIndexed: Bool {
        lock.lock(); defer { lock.unlock() }
        return searcher != nil && !_stale
    }

    public var status: Status {
        lock.lock(); defer { lock.unlock() }
        if let s = searcher {
            return Status(backend: .index, entryCount: s.entryCount, indexPath: indexPath, stale: _stale)
        }
        return Status(backend: SearchFSFallback.available ? .searchfs : .none,
                      entryCount: 0, indexPath: indexPath, stale: _stale)
    }

    // MARK: - Search

    /// Run a query through the hybrid engine. Uses the index when it is loaded
    /// and fresh; otherwise falls back to searchfs(). `forceFallback` bypasses
    /// the index (useful for A/B comparison in the UI).
    public func search(_ query: String,
                       options: SearchOptions = SearchOptions(),
                       forceFallback: Bool = false) -> (hits: [SearchHit], backend: Backend) {
        lock.lock()
        let s = searcher
        let stale = _stale
        lock.unlock()

        if let s, !forceFallback, !stale {
            return (s.search(query, options: options), .index)
        }
        if SearchFSFallback.available {
            return (SearchFSFallback.search(query, options: options), .searchfs)
        }
        // Last resort: a stale index still beats nothing.
        if let s {
            return (s.search(query, options: options), .index)
        }
        return ([], .none)
    }

    // MARK: - FSEvents incremental freshness

    /// Begin watching `roots` for changes; marks the index stale on any change
    /// and invokes `onStale` (debounced by FSEvents latency) so the owner can
    /// schedule a rebuild.
    public func startWatching(onStale: @escaping () -> Void = {}) {
        stopWatching()
        let w = FSEventsWatcher(paths: roots, latency: 3.0) { [weak self] in
            guard let self else { return }
            self.lock.lock(); self._stale = true; self.lock.unlock()
            onStale()
        }
        watcher = w
        w.start()
    }

    public func stopWatching() {
        watcher?.stop()
        watcher = nil
    }
}

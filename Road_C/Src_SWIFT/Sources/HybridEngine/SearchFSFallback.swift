import Foundation
import CSearchFS

/// Swift wrapper over the CSearchFS shim. This is the *fallback* path of the
/// hybrid engine: used when the mmap index is missing/corrupt, guaranteeing
/// 100%-fresh (if slower) results straight from the volume catalog.
public enum SearchFSFallback {

    /// Run searchfs() over `/` and, on Catalina+, `/System/Volumes/Data`.
    /// Collects up to `options.limit` unique paths that fuzzy-match locally so
    /// the ordering matches the index engine as closely as possible.
    public static func search(_ query: String, options: SearchOptions = SearchOptions()) -> [SearchHit] {
        let term = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !term.isEmpty else { return [] }

        // The C `enum { CSFS_* }` constants import into Swift as plain Int32, so
        // combine as integers and cast to the flags type (matches Road_A).
        var rawFlags: Int32 = Int32(CSFS_PARTIAL)
        if !options.dirsOnly  { rawFlags |= Int32(CSFS_MATCH_FILES) }
        if !options.filesOnly { rawFlags |= Int32(CSFS_MATCH_DIRS) }
        let opts = csfs_options_t(UInt32(bitPattern: rawFlags))

        var patLower = Array(query.utf8)
        for i in patLower.indices { patLower[i] = asciiLower(patLower[i]) }
        let collector = SearchFSCollector(limit: options.limit, pat: patLower)

        // A C function pointer must be non-capturing: everything it needs is
        // reached through the opaque `ctx` (the Collector) or via qualified
        // static/global calls — no local types, no captured locals.
        let cb: csfs_result_cb = { cPath, ctx in
            guard let cPath, let ctx else { return 1 }
            let c = Unmanaged<SearchFSCollector>.fromOpaque(ctx).takeUnretainedValue()
            let path = String(cString: cPath)
            if c.seen.insert(path).inserted {
                var isDirObjC: ObjCBool = false
                let exists = FileManager.default.fileExists(atPath: path, isDirectory: &isDirObjC)
                let isDir = exists && isDirObjC.boolValue
                let score = SearchFSFallback.fuzzyScoreOf(path: path, pat: c.pat)
                c.hits.append(SearchHit(path: path, isDir: isDir, score: score))
            }
            return c.hits.count >= c.limit ? 0 : 1   // 0 stops the C loop early
        }

        let ctx = Unmanaged.passUnretained(collector).toOpaque()
        _ = csfs_search("/", query, opts, cb, ctx)
        if collector.hits.count < options.limit && csfs_data_volume_available() != 0 {
            _ = csfs_search("/System/Volumes/Data", query, opts, cb, ctx)
        }

        var hits = collector.hits
        hits.sort { $0.score != $1.score ? $0.score > $1.score : $0.path.count < $1.path.count }
        if hits.count > options.limit { hits.removeLast(hits.count - options.limit) }
        return hits
    }

    /// Is catalog search usable on this machine at all?
    public static var available: Bool {
        csfs_volume_supports_searchfs("/") != 0
    }

    /// Score a full path's basename against the pattern for ordering parity with
    /// the index engine.
    private static func fuzzyScoreOf(path: String, pat: [UInt8]) -> Int {
        var bytes = Array(path.utf8)
        for i in bytes.indices { bytes[i] = asciiLower(bytes[i]) }
        let bnStart = (bytes.lastIndex(of: 0x2F).map { $0 + 1 }) ?? 0
        let bnSlice = Array(bytes[bnStart...])
        let bounds = computeBoundaries(bytes[bnStart...])
        return bnSlice.withUnsafeBufferPointer { text in
            pat.withUnsafeBufferPointer { p in
                FuzzyScore.score(pattern: p, text: text, boundaries: bounds, boundariesOffset: 0)?.score ?? 0
            }
        }
    }
}

/// Collector state threaded through the searchfs() C callback via an opaque
/// context pointer. Declared at file scope (not nested) so the C function
/// pointer that references it stays non-capturing.
private final class SearchFSCollector {
    var hits: [SearchHit] = []
    var seen = Set<String>()
    let limit: Int
    let pat: [UInt8]
    init(limit: Int, pat: [UInt8]) { self.limit = limit; self.pat = pat }
}

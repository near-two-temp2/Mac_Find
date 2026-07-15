import Foundation

/// A single search result.
public struct SearchResult: Identifiable, Hashable {
    public let id: Int          // entry index in the index
    public let path: String
    public let isDir: Bool
    public let score: Int

    public init(id: Int, path: String, isDir: Bool, score: Int) {
        self.id = id
        self.path = path
        self.isDir = isDir
        self.score = score
    }
}

public struct SearchOptions {
    public var maxResults: Int
    public var filesOnly: Bool
    public var dirsOnly: Bool
    /// Optional extension filter (without the dot), e.g. "swift".
    public var ext: String?

    public init(maxResults: Int = 500, filesOnly: Bool = false, dirsOnly: Bool = false, ext: String? = nil) {
        self.maxResults = maxResults
        self.filesOnly = filesOnly
        self.dirsOnly = dirsOnly
        self.ext = ext
    }
}

/// Two-phase search over a mmap-backed index.
public final class SearchEngine {
    public let index: MappedIndex

    public init(index: MappedIndex) {
        self.index = index
    }

    public convenience init(indexURL: URL) throws {
        self.init(index: try MappedIndex(url: indexURL))
    }

    /// Run a query. Empty query returns the first `maxResults` entries (browse).
    public func search(_ query: String, options: SearchOptions = SearchOptions()) -> [SearchResult] {
        let n = index.count
        guard n > 0 else { return [] }

        let patternBytes = Array(query.utf8).map(asciiLower)
        let queryMask = Bitmask.compute(patternBytes)

        // Phase 1: parallel candidate filtering.
        let survivors = phase1(patternBytes: patternBytes, queryMask: queryMask, options: options)

        // Phase 2: fuzzy score survivors, then rank.
        return phase2(candidates: survivors, patternBytes: patternBytes, options: options)
    }

    // MARK: - Phase 1

    private func phase1(patternBytes: [UInt8], queryMask: UInt64, options: SearchOptions) -> [Int] {
        let n = index.count
        let emptyQuery = patternBytes.isEmpty

        // Bucketed collection to avoid a shared lock during concurrentPerform.
        let coreCount = max(1, ProcessInfo.processInfo.activeProcessorCount)
        let chunkSize = (n + coreCount - 1) / coreCount
        let chunks = (n + chunkSize - 1) / chunkSize

        var buckets = [[Int]](repeating: [], count: chunks)

        buckets.withUnsafeMutableBufferPointer { bucketPtr in
            DispatchQueue.concurrentPerform(iterations: chunks) { c in
                let start = c * chunkSize
                let end = min(start + chunkSize, n)
                var local: [Int] = []
                local.reserveCapacity((end - start) / 8)

                for i in start..<end {
                    // Type filter.
                    let dir = index.isDir(i)
                    if options.filesOnly && dir { continue }
                    if options.dirsOnly && !dir { continue }

                    // Extension filter (exact ID match if we can resolve it, else
                    // fall back to letting Phase 2 handle it).
                    // (Extension IDs are per-build; the CLI resolves the query ext
                    // to bytes and Phase 2 substring covers correctness.)

                    if !emptyQuery {
                        // Bitmask prefilter: O(1) reject.
                        if !Bitmask.contains(entry: index.masks[i], query: queryMask) { continue }
                    }
                    local.append(i)
                }
                bucketPtr[c] = local
            }
        }

        var survivors: [Int] = []
        survivors.reserveCapacity(buckets.reduce(0) { $0 + $1.count })
        for b in buckets { survivors.append(contentsOf: b) }
        return survivors
    }

    // MARK: - Phase 2

    private func phase2(candidates: [Int], patternBytes: [UInt8], options: SearchOptions) -> [SearchResult] {
        let extFilter = options.ext?.lowercased()

        // Empty query: just take a browse slice, no scoring.
        if patternBytes.isEmpty {
            var out: [SearchResult] = []
            for i in candidates.prefix(options.maxResults) {
                if let extFilter, !hasExtension(i, extFilter) { continue }
                out.append(makeResult(i, score: 0))
                if out.count >= options.maxResults { break }
            }
            return out
        }

        // Score in parallel; each candidate scored against its basename first
        // (preferred) then full path.
        let scored = patternBytes.withUnsafeBufferPointer { pat -> [(Int, Int)] in
            let coreCount = max(1, ProcessInfo.processInfo.activeProcessorCount)
            let chunkSize = max(1, (candidates.count + coreCount - 1) / coreCount)
            let chunks = (candidates.count + chunkSize - 1) / chunkSize
            var buckets = [[(Int, Int)]](repeating: [], count: max(1, chunks))

            buckets.withUnsafeMutableBufferPointer { bucketPtr in
                DispatchQueue.concurrentPerform(iterations: max(1, chunks)) { c in
                    let start = c * chunkSize
                    let end = min(start + chunkSize, candidates.count)
                    var local: [(Int, Int)] = []
                    for k in start..<end {
                        let i = candidates[k]
                        if let extFilter, !hasExtension(i, extFilter) { continue }

                        let bn = index.basenameBytes(i)
                        let bnStart = Int(index.bnStarts[i])
                        // Prefer a basename match; boundaries are basename-relative.
                        if let mBN = FuzzyScorer.score(
                            pattern: pat, text: bn,
                            boundaries: index.bnBoundaries[i], boundariesOffset: 0
                        ) {
                            local.append((i, mBN.score + 20)) // slight basename preference
                        } else {
                            let full = index.pathBytes(i)
                            if let mFull = FuzzyScorer.score(
                                pattern: pat, text: full,
                                boundaries: index.bnBoundaries[i], boundariesOffset: bnStart
                            ) {
                                local.append((i, mFull.score))
                            }
                        }
                    }
                    bucketPtr[c] = local
                }
            }
            return buckets.flatMap { $0 }
        }

        // Sort by score desc, then shorter path, then path asc for stability.
        let ranked = scored.sorted { a, b in
            if a.1 != b.1 { return a.1 > b.1 }
            let la = Int(index.byteLengths[a.0]), lb = Int(index.byteLengths[b.0])
            if la != lb { return la < lb }
            return a.0 < b.0
        }

        var out: [SearchResult] = []
        out.reserveCapacity(min(options.maxResults, ranked.count))
        for (i, s) in ranked.prefix(options.maxResults) {
            out.append(makeResult(i, score: s))
        }
        return out
    }

    // MARK: - Helpers

    private func hasExtension(_ i: Int, _ ext: String) -> Bool {
        let bn = index.basenameBytes(i)
        let arr = Array(bn)
        guard let e = ExtensionTable.extractExtension(arr[...]) else { return false }
        return e == ext
    }

    private func makeResult(_ i: Int, score: Int) -> SearchResult {
        SearchResult(id: i, path: index.pathString(i), isDir: index.isDir(i), score: score)
    }
}

import Foundation

/// A single search hit surfaced to the UI / CLI.
public struct SearchHit: Sendable, Hashable {
    public let path: String
    public let isDir: Bool
    public let score: Int
    public init(path: String, isDir: Bool, score: Int) {
        self.path = path
        self.isDir = isDir
        self.score = score
    }
}

/// Options controlling a query.
public struct SearchOptions: Sendable {
    public var filesOnly: Bool
    public var dirsOnly: Bool
    public var limit: Int
    public init(filesOnly: Bool = false, dirsOnly: Bool = false, limit: Int = 500) {
        self.filesOnly = filesOnly
        self.dirsOnly = dirsOnly
        self.limit = limit
    }
}

/// Errors distinguishing "index unusable" (→ caller should fall back to
/// searchfs) from genuine IO failures.
public enum IndexError: Error {
    case missing
    case corrupt(String)
}

/// mmap-backed reader over a `.idx` file. Binds the mapped region straight to
/// typed pointers and runs the two-phase search:
///   Phase 1  parallel bitmask + ext + type prefilter (concurrentPerform)
///   Phase 2  fzf scoring on survivors, top-`limit` by score
public final class IndexSearcher {

    // Mapped region.
    private let base: UnsafeRawPointer
    private let mapLen: Int

    // Header-derived counts.
    public let entryCount: Int

    // Bound parallel-array pointers into the mapped region.
    private let masks: UnsafePointer<UInt64>
    private let bnMasks: UnsafePointer<UInt64>
    private let bnBounds: UnsafePointer<UInt64>
    private let loOffsets: UnsafePointer<UInt32>
    private let loLengths: UnsafePointer<UInt16>
    private let bnStarts: UnsafePointer<UInt16>
    private let origOffsets: UnsafePointer<UInt32>
    private let origLengths: UnsafePointer<UInt16>
    private let extIDs: UnsafePointer<UInt16>
    private let flags: UnsafePointer<UInt8>
    private let loBlob: UnsafePointer<UInt8>      // lowercased bytes (matching)
    private let origBlob: UnsafePointer<UInt8>    // original-case bytes (display)

    /// extension name → ID, recovered from the extension blob.
    private let extLookup: [String: UInt16]

    /// Flat bonus added to every basename match so a real filename hit always
    /// outranks an incidental match found deeper in a parent directory. Larger
    /// than any single fzf/tier delta so the ordering is: basename-exact >
    /// basename-anything > whole-path.
    static let basenameBoost = 1_000_000

    /// mmap + validate the index file. Throws `IndexError.missing/corrupt` when
    /// the file is absent or unusable so the hybrid layer can fall back.
    public init(path: String) throws {
        let fd = open(path, O_RDONLY)
        if fd < 0 {
            throw errno == ENOENT ? IndexError.missing : IndexError.corrupt("open failed (errno \(errno))")
        }
        defer { close(fd) }

        var st = stat()
        guard fstat(fd, &st) == 0 else { throw IndexError.corrupt("fstat failed") }
        let len = Int(st.st_size)
        guard len >= IndexFormat.headerSize else { throw IndexError.corrupt("too small") }

        guard let mapped = mmap(nil, len, PROT_READ, MAP_PRIVATE, fd, 0), mapped != MAP_FAILED else {
            throw IndexError.corrupt("mmap failed")
        }
        let raw = UnsafeRawPointer(mapped)

        // Parse + validate header.
        let magic = raw.loadUnaligned(fromByteOffset: 0, as: UInt64.self)
        guard magic == IndexFormat.magic else {
            munmap(mapped, len)
            throw IndexError.corrupt("bad magic")
        }
        let version = raw.loadUnaligned(fromByteOffset: 8, as: UInt32.self)
        guard version == IndexFormat.version else {
            munmap(mapped, len)
            throw IndexError.corrupt("version \(version)")
        }
        let n = Int(raw.loadUnaligned(fromByteOffset: 12, as: UInt32.self))
        let loBytes = Int(raw.loadUnaligned(fromByteOffset: 16, as: UInt64.self))
        let origBytes = Int(raw.loadUnaligned(fromByteOffset: 24, as: UInt64.self))
        let extBlobLen = Int(raw.loadUnaligned(fromByteOffset: 32, as: UInt32.self))

        // Walk the regions in write order, honouring the 8-byte padding.
        func region(_ cursor: inout Int, count: Int, stride: Int) -> Int {
            let start = cursor
            cursor = IndexFormat.align8(cursor + count * stride)
            return start
        }

        var cursor = IndexFormat.headerSize
        let oMasks    = region(&cursor, count: n, stride: 8)
        let oBnMasks  = region(&cursor, count: n, stride: 8)
        let oBnBnds   = region(&cursor, count: n, stride: 8)
        let oLoOffs   = region(&cursor, count: n, stride: 4)
        let oLoLens   = region(&cursor, count: n, stride: 2)
        let oBnStart  = region(&cursor, count: n, stride: 2)
        let oOrigOffs = region(&cursor, count: n, stride: 4)
        let oOrigLens = region(&cursor, count: n, stride: 2)
        let oExtIDs   = region(&cursor, count: n, stride: 2)
        let oFlags    = region(&cursor, count: n, stride: 1)
        cursor = IndexFormat.align8(cursor)
        let oExtBlob = cursor
        cursor = IndexFormat.align8(cursor + extBlobLen)
        let oLoBlob = cursor
        cursor = IndexFormat.align8(cursor + loBytes)
        let oOrigBlob = cursor

        guard oOrigBlob + origBytes <= len else {
            munmap(mapped, len)
            throw IndexError.corrupt("truncated: need \(oOrigBlob + origBytes), have \(len)")
        }

        self.base = raw
        self.mapLen = len
        self.entryCount = n
        self.masks       = raw.advanced(by: oMasks).assumingMemoryBound(to: UInt64.self)
        self.bnMasks     = raw.advanced(by: oBnMasks).assumingMemoryBound(to: UInt64.self)
        self.bnBounds    = raw.advanced(by: oBnBnds).assumingMemoryBound(to: UInt64.self)
        self.loOffsets   = raw.advanced(by: oLoOffs).assumingMemoryBound(to: UInt32.self)
        self.loLengths   = raw.advanced(by: oLoLens).assumingMemoryBound(to: UInt16.self)
        self.bnStarts    = raw.advanced(by: oBnStart).assumingMemoryBound(to: UInt16.self)
        self.origOffsets = raw.advanced(by: oOrigOffs).assumingMemoryBound(to: UInt32.self)
        self.origLengths = raw.advanced(by: oOrigLens).assumingMemoryBound(to: UInt16.self)
        self.extIDs      = raw.advanced(by: oExtIDs).assumingMemoryBound(to: UInt16.self)
        self.flags       = raw.advanced(by: oFlags).assumingMemoryBound(to: UInt8.self)
        self.loBlob      = raw.advanced(by: oLoBlob).assumingMemoryBound(to: UInt8.self)
        self.origBlob    = raw.advanced(by: oOrigBlob).assumingMemoryBound(to: UInt8.self)

        // Recover the extension table from its blob.
        var lookup: [String: UInt16] = [:]
        let extPtr = raw.advanced(by: oExtBlob).assumingMemoryBound(to: UInt8.self)
        var id: UInt16 = 0
        var start = 0
        var i = 0
        while i < extBlobLen {
            if extPtr[i] == 0 {
                if i > start {
                    let name = String(decoding: UnsafeBufferPointer(start: extPtr + start, count: i - start), as: UTF8.self)
                    lookup[name] = id
                }
                id &+= 1
                start = i + 1
            }
            i += 1
        }
        self.extLookup = lookup
    }

    deinit {
        munmap(UnsafeMutableRawPointer(mutating: base), mapLen)
    }

    // MARK: - Query

    /// Run the two-phase search. `query` is matched fuzzily against the
    /// basename; if it contains a '/', the whole path is used as the haystack.
    public func search(_ query: String, options: SearchOptions = SearchOptions()) -> [SearchHit] {
        var pat = Array(query.utf8)
        for i in pat.indices { pat[i] = asciiLower(pat[i]) }
        if pat.isEmpty { return recents(options: options) }

        let matchWholePath = pat.contains(0x2F)
        let queryMask = Bitmask.compute(pat)

        // Optional extension constraint from a trailing ".ext" query token.
        let extConstraint = extConstraintID(pat)

        // Phase 1 (parallel) → Phase 2 (fzf), collected per-shard then merged.
        let cores = max(1, ProcessInfo.processInfo.activeProcessorCount)
        let shardCount = min(cores, max(1, entryCount / 4096 + 1))
        let shardSize = (entryCount + shardCount - 1) / max(1, shardCount)

        // Per-shard result buckets (avoids cross-thread contention).
        let buckets = UnsafeMutablePointer<[SearchHit]>.allocate(capacity: shardCount)
        for k in 0..<shardCount { (buckets + k).initialize(to: []) }
        defer {
            for k in 0..<shardCount { (buckets + k).deinitialize(count: 1) }
            buckets.deallocate()
        }

        pat.withUnsafeBufferPointer { patBuf in
            DispatchQueue.concurrentPerform(iterations: shardCount) { shard in
                let lo = shard * shardSize
                let hi = min(lo + shardSize, entryCount)
                if lo >= hi { return }
                var local: [SearchHit] = []
                for i in lo..<hi {
                    if let hit = scoreEntry(i, pat: patBuf, queryMask: queryMask,
                                            matchWholePath: matchWholePath,
                                            extConstraint: extConstraint,
                                            options: options) {
                        local.append(hit)
                    }
                }
                buckets[shard] = local
            }
        }

        var all: [SearchHit] = []
        for k in 0..<shardCount { all.append(contentsOf: buckets[k]) }

        all.sort { $0.score != $1.score ? $0.score > $1.score : $0.path.count < $1.path.count }
        if all.count > options.limit { all.removeLast(all.count - options.limit) }
        return all
    }

    /// Score one entry through Phase 1 filters and Phase 2 fzf. Returns nil if
    /// the entry is filtered out or doesn't fuzzy-match.
    @inline(__always)
    private func scoreEntry(_ i: Int,
                            pat: UnsafeBufferPointer<UInt8>,
                            queryMask: UInt64,
                            matchWholePath: Bool,
                            extConstraint: UInt16?,
                            options: SearchOptions) -> SearchHit? {
        let isDir = flags[i] & 1 != 0
        if options.filesOnly && isDir { return nil }
        if options.dirsOnly && !isDir { return nil }

        // Phase 1a: bitmask prefilter against the relevant mask.
        let entryMask = matchWholePath ? masks[i] : bnMasks[i]
        if !Bitmask.contains(entry: entryMask, query: queryMask) { return nil }

        // Phase 1b: extension constraint.
        if let want = extConstraint, extIDs[i] != want { return nil }

        // Phase 2: fzf over basename (or whole path), matching the lowercased blob.
        let loOff = Int(loOffsets[i])
        let loLen = Int(loLengths[i])
        let bnStart = Int(bnStarts[i])
        if bnStart > loLen { return nil }   // defensive: corrupt/stale entry

        let hitScore: Int
        if matchWholePath {
            // Query contains '/': match against the whole path. Word boundaries
            // over a full path are dominated by the '/' separators, so pass the
            // whole-path bounds (offset 0) rather than the basename-only bitmap.
            let textPtr = loBlob + loOff
            guard let r = FuzzyScore.scoreRanked(
                pattern: pat,
                text: UnsafeBufferPointer(start: textPtr, count: loLen),
                boundaries: wholePathBoundaries(base: textPtr, len: loLen),
                boundariesOffset: 0) else { return nil }
            hitScore = r.score
        } else {
            // Basename-only: a query without '/' matches the *filename*, never a
            // parent directory. Phase-1a already prefiltered on the basename mask
            // (`bnMasks[i]`), so parent-path incidental hits are excluded here by
            // construction — that's what kills the fzf noise item ① called out.
            // The large basename boost keeps these above any '/'-query path hits.
            let bnPtr = loBlob + loOff + bnStart
            let bnLen = loLen - bnStart
            guard let r = FuzzyScore.scoreRanked(
                pattern: pat,
                text: UnsafeBufferPointer(start: bnPtr, count: bnLen),
                boundaries: bnBounds[i], boundariesOffset: 0) else { return nil }
            hitScore = r.score + IndexSearcher.basenameBoost
        }

        // Display the original-case path.
        let path = String(decoding: UnsafeBufferPointer(start: origBlob + Int(origOffsets[i]),
                                                        count: Int(origLengths[i])), as: UTF8.self)
        return SearchHit(path: path, isDir: isDir, score: hitScore)
    }

    /// Word-boundary bitmap over the first 64 bytes of a whole path: bit i is set
    /// when byte i starts a word (index 0, or right after a `/ . - _ space`, or a
    /// digit-run start). Mirrors `computeBoundaries` but works over a raw pointer.
    @inline(__always)
    private func wholePathBoundaries(base: UnsafePointer<UInt8>, len: Int) -> UInt64 {
        var mask: UInt64 = 0
        var prev: UInt8 = 0x2F   // pretend a leading separator so index 0 counts
        let n = min(len, 64)
        var i = 0
        while i < n {
            let b = base[i]
            let sep = (prev == 0x2F || prev == 0x2E || prev == 0x2D || prev == 0x5F || prev == 0x20)
            let digitStart = (b >= 0x30 && b <= 0x39) && !(prev >= 0x30 && prev <= 0x39)
            if sep || digitStart { mask |= (1 << UInt64(i)) }
            prev = b
            i += 1
        }
        return mask
    }

    /// Extract an extension constraint if the query is a bare ".ext" or "*.ext".
    private func extConstraintID(_ pat: [UInt8]) -> UInt16? {
        guard let dot = pat.lastIndex(of: 0x2E) else { return nil }
        // Only treat as a constraint when the dot is at/near the start (".swift"
        // or "*.swift"), i.e. the query is essentially just an extension.
        let prefixOK = dot == 0 || (dot == 1 && pat[0] == 0x2A)
        guard prefixOK else { return nil }
        let ext = String(decoding: pat[(dot + 1)...], as: UTF8.self)
        return extLookup[ext]
    }

    /// With an empty query, surface the first `limit` entries as "recents-ish"
    /// placeholder content so the UI isn't blank.
    private func recents(options: SearchOptions) -> [SearchHit] {
        var out: [SearchHit] = []
        let cap = min(options.limit, entryCount)
        var i = 0
        while out.count < cap && i < entryCount {
            let isDir = flags[i] & 1 != 0
            if !(options.filesOnly && isDir) && !(options.dirsOnly && !isDir) {
                let path = String(decoding: UnsafeBufferPointer(start: origBlob + Int(origOffsets[i]),
                                                                count: Int(origLengths[i])), as: UTF8.self)
                out.append(SearchHit(path: path, isDir: isDir, score: 0))
            }
            i += 1
        }
        return out
    }
}

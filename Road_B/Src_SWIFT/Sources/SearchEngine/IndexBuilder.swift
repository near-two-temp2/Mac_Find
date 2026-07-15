import Foundation

/// Walks the filesystem and serializes a binary index to disk.
///
/// Uses `FileManager.enumerator` for portability. Cling uses `fts_open` with
/// `FTS_NOSTAT` for speed; that's noted as a TODO here (see README). For the
/// first cut, correctness + the mmap/bitmask/fzf query path is what matters.
public final class IndexBuilder {

    public struct Progress {
        public var scanned: Int
        public var currentPath: String
    }

    public init() {}

    /// Build an index of `root` and write it to `outputURL`.
    /// - Returns: number of entries written.
    @discardableResult
    public func build(
        root: URL,
        outputURL: URL,
        includeHidden: Bool = false,
        progress: ((Progress) -> Void)? = nil
    ) throws -> Int {
        var entries: [IndexEntry] = []
        entries.reserveCapacity(1 << 16)
        let extTable = ExtensionTable()

        let fm = FileManager.default
        var options: FileManager.DirectoryEnumerationOptions = [.skipsPackageDescendants]
        if !includeHidden { options.insert(.skipsHiddenFiles) }

        guard let enumerator = fm.enumerator(
            at: root,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: options,
            errorHandler: { _, _ in true } // keep going past permission errors
        ) else {
            throw IndexError.enumerationFailed(root.path)
        }

        var scanned = 0
        for case let fileURL as URL in enumerator {
            let path = fileURL.path
            var isDir = false
            if let vals = try? fileURL.resourceValues(forKeys: [.isDirectoryKey]) {
                isDir = vals.isDirectory ?? false
            }
            let entry = makeEntry(path: path, isDir: isDir, extTable: extTable)
            entries.append(entry)

            scanned += 1
            if let progress, scanned % 4096 == 0 {
                progress(Progress(scanned: scanned, currentPath: path))
            }
        }
        progress?(Progress(scanned: scanned, currentPath: root.path))

        try serialize(entries: entries, to: outputURL)
        return entries.count
    }

    /// Build an index from an explicit list of paths (used by tests / CI so we
    /// don't depend on the real filesystem layout).
    @discardableResult
    public func build(paths: [(path: String, isDir: Bool)], outputURL: URL) throws -> Int {
        let extTable = ExtensionTable()
        let entries = paths.map { makeEntry(path: $0.path, isDir: $0.isDir, extTable: extTable) }
        try serialize(entries: entries, to: outputURL)
        return entries.count
    }

    // MARK: - Entry construction

    func makeEntry(path: String, isDir: Bool, extTable: ExtensionTable) -> IndexEntry {
        var bytes = Array(path.utf8).map(asciiLower)
        // Guard against absurdly long paths overflowing the UInt16 length field.
        if bytes.count > 0xFFFF { bytes = Array(bytes.prefix(0xFFFF)) }
        let bnStart = lastSlashIndex(bytes).map { $0 + 1 } ?? 0
        let extID = extTable.intern(basenameLower: bytes[bnStart...])
        return IndexEntry(pathLower: bytes, basenameStart: bnStart, extID: extID, isDir: isDir)
    }

    private func lastSlashIndex(_ bytes: [UInt8]) -> Int? {
        var i = bytes.count - 1
        while i >= 0 {
            if bytes[i] == 0x2F { return i }
            i -= 1
        }
        return nil
    }

    // MARK: - Serialization

    func serialize(entries: [IndexEntry], to url: URL) throws {
        let n = entries.count

        // First pass: pack the path blob and record offsets.
        var blob: [UInt8] = []
        blob.reserveCapacity(n * 40)
        var byteOffsets = [UInt32](repeating: 0, count: n)
        var byteLengths = [UInt16](repeating: 0, count: n)
        for i in 0..<n {
            byteOffsets[i] = UInt32(truncatingIfNeeded: blob.count)
            byteLengths[i] = UInt16(truncatingIfNeeded: entries[i].pathLower.count)
            blob.append(contentsOf: entries[i].pathLower)
        }

        var masks = [UInt64](repeating: 0, count: n)
        var bnMasks = [UInt64](repeating: 0, count: n)
        var bnBoundaries = [UInt64](repeating: 0, count: n)
        var bnStarts = [UInt16](repeating: 0, count: n)
        var extIDs = [UInt16](repeating: 0, count: n)
        var flags = [UInt8](repeating: 0, count: n)

        for i in 0..<n {
            let e = entries[i]
            masks[i] = Bitmask.compute(e.pathLower)
            let bn = e.pathLower[e.basenameStart...]
            bnMasks[i] = Bitmask.compute(Array(bn))
            bnBoundaries[i] = computeBoundaries(bn)
            bnStarts[i] = UInt16(truncatingIfNeeded: e.basenameStart)
            extIDs[i] = e.extID
            flags[i] = e.isDir ? 1 : 0
        }

        var data = Data()
        data.reserveCapacity(IndexFormat.headerSize + n * 40 + blob.count)

        // Header
        appendLE(&data, IndexFormat.magic)
        appendLE(&data, IndexFormat.version)
        appendLE(&data, UInt32(truncatingIfNeeded: n))
        appendLE(&data, UInt64(blob.count))
        appendLE(&data, UInt64(0)) // reserved

        // Parallel arrays
        masks.withUnsafeBytes { data.append(contentsOf: $0) }
        bnMasks.withUnsafeBytes { data.append(contentsOf: $0) }
        bnBoundaries.withUnsafeBytes { data.append(contentsOf: $0) }
        byteOffsets.withUnsafeBytes { data.append(contentsOf: $0) }
        byteLengths.withUnsafeBytes { data.append(contentsOf: $0) }
        bnStarts.withUnsafeBytes { data.append(contentsOf: $0) }
        extIDs.withUnsafeBytes { data.append(contentsOf: $0) }
        flags.withUnsafeBytes { data.append(contentsOf: $0) }

        // Blob
        blob.withUnsafeBytes { data.append(contentsOf: $0) }

        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try data.write(to: url, options: .atomic)
    }
}

public enum IndexError: Error, CustomStringConvertible {
    case enumerationFailed(String)
    case mmapFailed(String)
    case badMagic
    case truncated

    public var description: String {
        switch self {
        case .enumerationFailed(let p): return "failed to enumerate: \(p)"
        case .mmapFailed(let p): return "mmap failed: \(p)"
        case .badMagic: return "index has bad magic (not a MacHaiFindB index)"
        case .truncated: return "index file is truncated / corrupt"
        }
    }
}

@inline(__always)
func appendLE(_ data: inout Data, _ value: UInt64) {
    var v = value.littleEndian
    withUnsafeBytes(of: &v) { data.append(contentsOf: $0) }
}

@inline(__always)
func appendLE(_ data: inout Data, _ value: UInt32) {
    var v = value.littleEndian
    withUnsafeBytes(of: &v) { data.append(contentsOf: $0) }
}

import Foundation

/// Builds the binary index by walking one or more root directories and
/// serialising the parallel-array format defined in `BinaryIndex.swift`.
///
/// The walk uses `FileManager.enumerator` (Foundation-portable) rather than
/// `fts_open`; for the flagship's initial scan this is adequate, and the
/// searchfs()-backed initial scan noted in the analysis is left as a TODO
/// (see README) so the pure-logic engine stays testable off-device.
public final class IndexBuilder {

    public struct Options {
        public var includeHidden: Bool
        public var maxEntries: Int
        /// Directory basenames pruned during the walk (never descended into).
        public var prunedDirs: Set<String>

        public init(includeHidden: Bool = false,
                    maxEntries: Int = 5_000_000,
                    prunedDirs: Set<String> = IndexBuilder.defaultPrunedDirs) {
            self.includeHidden = includeHidden
            self.maxEntries = maxEntries
            self.prunedDirs = prunedDirs
        }
    }

    /// Directories that never carry user-searchable content and only bloat the
    /// index (mirrors Cling's default ignore groups, trimmed).
    public static let defaultPrunedDirs: Set<String> = [
        ".git", "node_modules", ".build", "DerivedData", ".Trash",
        "Caches", ".cache", "Pods", ".gradle", "target", "__pycache__",
    ]

    public init() {}

    /// Walk `roots`, collect entries, and write the binary index to `outPath`.
    /// Returns the number of entries written.
    @discardableResult
    public func build(roots: [String], to outPath: String, options: Options = Options()) throws -> Int {
        var entries: [IndexEntry] = []
        entries.reserveCapacity(1 << 16)
        let extTable = ExtensionTable()

        for root in roots {
            collect(root: root, into: &entries, extTable: extTable, options: options)
            if entries.count >= options.maxEntries { break }
        }

        let data = serialize(entries: entries, extTable: extTable)
        try data.write(to: URL(fileURLWithPath: outPath), options: .atomic)
        return entries.count
    }

    // MARK: - Walk

    private func collect(root: String,
                         into entries: inout [IndexEntry],
                         extTable: ExtensionTable,
                         options: Options) {
        let fm = FileManager.default
        let rootURL = URL(fileURLWithPath: root)

        var opts: FileManager.DirectoryEnumerationOptions = [.skipsPackageDescendants]
        if !options.includeHidden { opts.insert(.skipsHiddenFiles) }

        guard let en = fm.enumerator(
            at: rootURL,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: opts,
            errorHandler: { _, _ in true }   // ignore unreadable subtrees
        ) else { return }

        for case let url as URL in en {
            if entries.count >= options.maxEntries { break }

            let name = url.lastPathComponent
            let isDir = (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory ?? false

            if isDir && options.prunedDirs.contains(name) {
                en.skipDescendants()
                continue
            }

            addEntry(path: url.path, isDir: isDir, into: &entries, extTable: extTable)
        }
    }

    /// Turn a single path into an IndexEntry: original bytes (for display) plus
    /// lowercased bytes and derived fields (for matching).
    func addEntry(path: String,
                  isDir: Bool,
                  into entries: inout [IndexEntry],
                  extTable: ExtensionTable) {
        let orig = Array(path.utf8)
        var lower = orig
        for i in lower.indices { lower[i] = asciiLower(lower[i]) }

        // Basename start = one past the last '/'.
        var bnStart = 0
        if let slash = lower.lastIndex(of: 0x2F) { bnStart = slash + 1 }

        let extID = extTable.intern(basenameLower: lower[bnStart...])
        entries.append(IndexEntry(pathLower: lower,
                                  pathOrig: orig,
                                  basenameStart: bnStart,
                                  extID: extID,
                                  isDir: isDir))
    }

    // MARK: - Serialize

    func serialize(entries: [IndexEntry], extTable: ExtensionTable) -> Data {
        let n = entries.count

        // Two path blobs: lowercased (matching) + original-case (display).
        var loBlob = [UInt8](); loBlob.reserveCapacity(n * 48)
        var origBlob = [UInt8](); origBlob.reserveCapacity(n * 48)

        var masks = [UInt64](repeating: 0, count: n)
        var bnMasks = [UInt64](repeating: 0, count: n)
        var bnBounds = [UInt64](repeating: 0, count: n)
        var loOffsets = [UInt32](repeating: 0, count: n)
        var loLengths = [UInt16](repeating: 0, count: n)
        var bnStarts = [UInt16](repeating: 0, count: n)
        var origOffsets = [UInt32](repeating: 0, count: n)
        var origLengths = [UInt16](repeating: 0, count: n)
        var extIDs = [UInt16](repeating: 0, count: n)
        var flags = [UInt8](repeating: 0, count: n)

        for (i, e) in entries.enumerated() {
            loOffsets[i] = UInt32(truncatingIfNeeded: loBlob.count)
            loLengths[i] = UInt16(truncatingIfNeeded: min(e.pathLower.count, Int(UInt16.max)))
            bnStarts[i] = UInt16(truncatingIfNeeded: min(e.basenameStart, Int(UInt16.max)))
            origOffsets[i] = UInt32(truncatingIfNeeded: origBlob.count)
            origLengths[i] = UInt16(truncatingIfNeeded: min(e.pathOrig.count, Int(UInt16.max)))
            extIDs[i] = e.extID
            flags[i] = e.isDir ? 1 : 0

            masks[i] = Bitmask.compute(e.pathLower)
            let bnSlice = e.pathLower[e.basenameStart...]
            bnMasks[i] = Bitmask.compute(Array(bnSlice))
            bnBounds[i] = computeBoundaries(bnSlice)

            loBlob.append(contentsOf: e.pathLower)
            origBlob.append(contentsOf: e.pathOrig)
        }

        // Extension-name blob: NUL-separated, in ID order.
        var extBlob = [UInt8]()
        for name in extTable.names {
            extBlob.append(contentsOf: Array(name.utf8))
            extBlob.append(0)
        }

        var out = Data()
        out.reserveCapacity(IndexFormat.headerSize + n * 48 + loBlob.count + origBlob.count + extBlob.count + 64)

        // Header (40 bytes).
        appendLE(&out, IndexFormat.magic)
        appendLE(&out, IndexFormat.version)
        appendLE(&out, UInt32(truncatingIfNeeded: n))
        appendLE(&out, UInt64(loBlob.count))
        appendLE(&out, UInt64(origBlob.count))
        appendLE(&out, UInt32(truncatingIfNeeded: extBlob.count))
        appendLE(&out, UInt32(0)) // reserved

        // Parallel arrays, each region padded to an 8-byte boundary.
        appendArray(&out, masks)
        appendArray(&out, bnMasks)
        appendArray(&out, bnBounds)
        appendArray(&out, loOffsets)
        appendArray(&out, loLengths)
        appendArray(&out, bnStarts)
        appendArray(&out, origOffsets)
        appendArray(&out, origLengths)
        appendArray(&out, extIDs)
        appendArray(&out, flags)

        padTo8(&out)
        out.append(contentsOf: extBlob)
        padTo8(&out)
        out.append(contentsOf: loBlob)
        padTo8(&out)
        out.append(contentsOf: origBlob)

        return out
    }

    // MARK: - Little-endian helpers

    private func appendLE(_ d: inout Data, _ v: UInt64) { var x = v.littleEndian; withUnsafeBytes(of: &x) { d.append(contentsOf: $0) } }
    private func appendLE(_ d: inout Data, _ v: UInt32) { var x = v.littleEndian; withUnsafeBytes(of: &x) { d.append(contentsOf: $0) } }

    private func appendArray<T>(_ d: inout Data, _ arr: [T]) {
        arr.withUnsafeBytes { d.append(contentsOf: $0) }
        padTo8(&d)
    }

    private func padTo8(_ d: inout Data) {
        let rem = d.count & 7
        if rem != 0 { d.append(contentsOf: [UInt8](repeating: 0, count: 8 - rem)) }
    }
}

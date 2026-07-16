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
        /// Hard ceiling on indexed entries. Defaults to `.max` (no practical cap)
        /// so full-disk coverage never drops `~/temp_test` the way a 50k limit did.
        /// Present only as a safety valve for pathological volumes.
        public var maxEntries: Int
        /// Directory basenames pruned during the walk (never descended into).
        public var prunedDirs: Set<String>
        /// When true (default), the walk stays on local (apfs/hfs) filesystems and
        /// refuses to cross into network / FUSE mounts. Guards against the rclone→
        /// Backblaze B2 mounts on this machine (burning API quota) and any other
        /// slow network volume. See `SEARCH_TEST_BASELINE.md`.
        public var skipNetworkVolumes: Bool

        public init(includeHidden: Bool = false,
                    maxEntries: Int = .max,
                    prunedDirs: Set<String> = IndexBuilder.defaultPrunedDirs,
                    skipNetworkVolumes: Bool = true) {
            self.includeHidden = includeHidden
            self.maxEntries = maxEntries
            self.prunedDirs = prunedDirs
            self.skipNetworkVolumes = skipNetworkVolumes
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

        // Refuse to even start on a non-local root (e.g. someone points a root at
        // an rclone mount): that would spin up network/B2 traffic immediately.
        let guardMounts = options.skipNetworkVolumes ? MountGuard() : nil
        if let g = guardMounts, !g.isLocal(path: root) { return }

        var opts: FileManager.DirectoryEnumerationOptions = [.skipsPackageDescendants]
        if !options.includeHidden { opts.insert(.skipsHiddenFiles) }

        guard let en = fm.enumerator(
            at: rootURL,
            includingPropertiesForKeys: [.isDirectoryKey, .volumeIsLocalKey],
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

            // Network / FUSE guard: don't descend into a directory that sits on a
            // non-local (network/FUSE) volume. This prunes ~/Library/CloudStorage,
            // /Volumes/Disk/h2-*, and any smb/nfs/afp/webdav mount — while still
            // following macOS's system→data firmlink (both are local apfs). Only
            // directories that are themselves mount points change volume, so the
            // check is effectively per-mount, not per-entry.
            if isDir, let g = guardMounts {
                if g.isExcludedPrefix(url.path) ||
                   (isMountPoint(url) && !g.isLocal(path: url.path)) {
                    en.skipDescendants()
                    continue
                }
            }

            addEntry(path: url.path, isDir: isDir, into: &entries, extTable: extTable)
        }
    }

    /// Cheap mount-point test: a directory is a mount root when its `st_dev`
    /// differs from its parent's. Only such dirs can change local/network status,
    /// so we run the (more expensive) `isLocal` probe just for them.
    private func isMountPoint(_ url: URL) -> Bool {
        var st = stat()
        guard lstat(url.path, &st) == 0 else { return false }
        let parent = url.deletingLastPathComponent().path
        var pst = stat()
        guard lstat(parent, &pst) == 0 else { return false }
        return st.st_dev != pst.st_dev
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

// MARK: - Network / FUSE mount guard

/// Classifies filesystem paths as local vs. network so the index walk never
/// touches network / FUSE volumes.
///
/// Two layers, per `SEARCH_TEST_BASELINE.md`:
///   1. Live probe: `getmntinfo` snapshots every mount; a mount counts as local
///      only if its `f_fstypename` is `apfs`/`hfs` **and** `MNT_LOCAL` is set.
///      Any `macfuse`/`nfs`/`smbfs`/`afpfs`/`webdav`/… mount is network.
///   2. Belt-and-braces: an explicit prefix denylist for this machine's known
///      rclone→B2 mounts and the CloudStorage FileProvider roots, so even if the
///      probe misreads them we still refuse to descend.
final class MountGuard {

    /// Local filesystem type names we index (everything else is treated as
    /// network / removable-remote and skipped).
    private static let localFSTypes: Set<String> = ["apfs", "hfs"]

    /// Lowercased mount points that are known to be network/FUSE on this host,
    /// plus the substrings that identify CloudStorage providers.
    private static let excludedPrefixes: [String] = [
        "/volumes/disk/h2-bu-01",
        "/volumes/disk/h2_bu_01_b2",
        "/volumes/disk/h2_open_rsh",
    ]
    private static let excludedSubstrings: [String] = [
        "/library/cloudstorage/googledrive-",
        "/library/cloudstorage/onedrive-",
        "/library/cloudstorage/",     // any other FileProvider vendor
    ]

    /// Non-local mount points discovered live at construction time (lowercased).
    private let networkMountPoints: Set<String>

    init() {
        var pts = Set<String>()
        var mntbufp: UnsafeMutablePointer<statfs>? = nil
        let n = getmntinfo(&mntbufp, MNT_NOWAIT)
        if n > 0, let buf = mntbufp {
            for i in 0..<Int(n) {
                var m = buf[i]
                let fstype = withUnsafeBytes(of: &m.f_fstypename) { raw -> String in
                    let p = raw.bindMemory(to: CChar.self).baseAddress!
                    return String(cString: p)
                }.lowercased()
                let mountPoint = withUnsafeBytes(of: &m.f_mntonname) { raw -> String in
                    let p = raw.bindMemory(to: CChar.self).baseAddress!
                    return String(cString: p)
                }
                let isLocalFlag = (m.f_flags & UInt32(MNT_LOCAL)) != 0
                let isLocal = isLocalFlag && Self.localFSTypes.contains(fstype)
                if !isLocal { pts.insert(mountPoint.lowercased()) }
            }
        }
        self.networkMountPoints = pts
    }

    /// True if `path` lives on a local (apfs/hfs) volume and isn't denylisted.
    func isLocal(path: String) -> Bool {
        if isExcludedPrefix(path) { return false }
        let lower = path.lowercased()
        // On a known network mount point (or below one)?
        for mp in networkMountPoints where mp != "/" {
            if lower == mp || lower.hasPrefix(mp + "/") { return false }
        }
        return true
    }

    /// Explicit denylist check (independent of the live probe).
    func isExcludedPrefix(_ path: String) -> Bool {
        let lower = path.lowercased()
        for p in Self.excludedPrefixes where lower == p || lower.hasPrefix(p + "/") { return true }
        for s in Self.excludedSubstrings where lower.contains(s) { return true }
        return false
    }
}

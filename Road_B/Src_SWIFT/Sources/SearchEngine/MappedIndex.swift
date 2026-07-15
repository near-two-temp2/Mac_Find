import Foundation

/// A read-only, mmap-backed view over an index file. All parallel arrays are
/// exposed as `UnsafeBufferPointer`s that point directly into the mapped region
/// — no copies. The mapping is released on `deinit`.
public final class MappedIndex {
    private let base: UnsafeRawPointer
    private let mappedLength: Int

    public let count: Int
    public let bytesCount: Int

    // Parallel-array pointers into the mapped region.
    public let masks: UnsafeBufferPointer<UInt64>
    public let bnMasks: UnsafeBufferPointer<UInt64>
    public let bnBoundaries: UnsafeBufferPointer<UInt64>
    public let byteOffsets: UnsafeBufferPointer<UInt32>
    public let byteLengths: UnsafeBufferPointer<UInt16>
    public let bnStarts: UnsafeBufferPointer<UInt16>
    public let extIDs: UnsafeBufferPointer<UInt16>
    public let flags: UnsafeBufferPointer<UInt8>
    public let blob: UnsafeBufferPointer<UInt8>

    public init(url: URL) throws {
        let fd = open(url.path, O_RDONLY)
        guard fd >= 0 else { throw IndexError.mmapFailed(url.path) }
        defer { close(fd) }

        var st = stat()
        guard fstat(fd, &st) == 0 else { throw IndexError.mmapFailed(url.path) }
        let length = Int(st.st_size)
        guard length >= IndexFormat.headerSize else { throw IndexError.truncated }

        guard let ptr = mmap(nil, length, PROT_READ, MAP_PRIVATE, fd, 0),
              ptr != MAP_FAILED else {
            throw IndexError.mmapFailed(url.path)
        }

        self.base = UnsafeRawPointer(ptr)
        self.mappedLength = length

        // Parse header.
        let magic = base.load(fromByteOffset: 0, as: UInt64.self).littleEndian
        guard magic == IndexFormat.magic else {
            munmap(ptr, length)
            throw IndexError.badMagic
        }
        let n = Int(base.load(fromByteOffset: 12, as: UInt32.self).littleEndian)
        let blobLen = Int(base.load(fromByteOffset: 16, as: UInt64.self).littleEndian)
        self.count = n
        self.bytesCount = blobLen

        // Bounds sanity check before we build any pointers.
        let expected = IndexFormat.headerSize
            + n * (8 + 8 + 8 + 4 + 2 + 2 + 2 + 1) // per-entry bytes
            + blobLen
        guard expected <= length else {
            munmap(ptr, length)
            throw IndexError.truncated
        }

        // Compute section offsets in the same order they were written.
        // `section` is a pure local closure over `mappedBase`/`off` only — it
        // never touches `self`, so we can call it while `self` is still being
        // initialized (Swift 6 forbids method calls on a partially-initialized
        // `self`).
        let mappedBase = UnsafeRawPointer(ptr)
        var off = IndexFormat.headerSize
        func section<T>(_ type: T.Type, _ elems: Int) -> UnsafeBufferPointer<T> {
            let p = mappedBase.advanced(by: off).assumingMemoryBound(to: T.self)
            off += elems * MemoryLayout<T>.stride
            return UnsafeBufferPointer(start: p, count: elems)
        }

        self.masks = section(UInt64.self, n)
        self.bnMasks = section(UInt64.self, n)
        self.bnBoundaries = section(UInt64.self, n)
        self.byteOffsets = section(UInt32.self, n)
        self.byteLengths = section(UInt16.self, n)
        self.bnStarts = section(UInt16.self, n)
        self.extIDs = section(UInt16.self, n)
        self.flags = section(UInt8.self, n)
        self.blob = section(UInt8.self, blobLen)
    }

    deinit {
        munmap(UnsafeMutableRawPointer(mutating: base), mappedLength)
    }

    /// Full lowercased path bytes for entry `i`.
    @inline(__always)
    public func pathBytes(_ i: Int) -> UnsafeBufferPointer<UInt8> {
        let start = Int(byteOffsets[i])
        let len = Int(byteLengths[i])
        return UnsafeBufferPointer(start: blob.baseAddress!.advanced(by: start), count: len)
    }

    /// Basename bytes for entry `i`.
    @inline(__always)
    public func basenameBytes(_ i: Int) -> UnsafeBufferPointer<UInt8> {
        let start = Int(byteOffsets[i]) + Int(bnStarts[i])
        let len = Int(byteLengths[i]) - Int(bnStarts[i])
        return UnsafeBufferPointer(start: blob.baseAddress!.advanced(by: start), count: max(0, len))
    }

    @inline(__always)
    public func isDir(_ i: Int) -> Bool { (flags[i] & 1) != 0 }

    /// Reconstruct the path as a Swift String (only for display of final hits).
    public func pathString(_ i: Int) -> String {
        let start = Int(byteOffsets[i])
        let len = Int(byteLengths[i])
        return String(decoding: blob[start..<(start + len)], as: UTF8.self)
    }
}

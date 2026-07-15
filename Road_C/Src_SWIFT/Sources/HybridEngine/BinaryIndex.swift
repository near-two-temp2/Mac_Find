import Foundation

/// On-disk / in-memory binary index (mmap-friendly, parallel-array layout).
///
/// Layout, all host-endian (little on the only platforms we target):
///
///   Header (40 bytes):
///     magic:          UInt64  "MHFINDC1" tag
///     version:        UInt32
///     entryCount:     UInt32
///     lowerBytesCount:UInt64  (size of the lowercased-path blob, for matching)
///     origBytesCount: UInt64  (size of the original-path blob, for display)
///     extBlobLen:     UInt32  (size of the extension-name blob, NUL-separated)
///     reserved:       UInt32
///
///   Parallel arrays (entryCount elements each, in this order, each region
///   individually 8-byte aligned so pointers can be bound directly):
///     masks[i]:        UInt64  path letter bitmask (over lowercased path)
///     bnMasks[i]:      UInt64  basename letter bitmask
///     bnBoundaries[i]: UInt64  word-boundary bitmap within the basename
///     loOffsets[i]:    UInt32  offset into the lowercased-path blob
///     loLengths[i]:    UInt16  lowercased-path byte length
///     bnStarts[i]:     UInt16  basename start offset within the path
///     origOffsets[i]:  UInt32  offset into the original-path blob
///     origLengths[i]:  UInt16  original-path byte length
///     extIDs[i]:       UInt16  interned extension id (0 = none)
///     flags[i]:        UInt8   bit0 = isDir
///
///   Extension-name blob: NUL-separated extension strings, in ID order starting
///     at ID 0 (the empty string). Lets a loaded index resolve a query
///     extension → ID without rebuilding.
///
///   Lower-path blob:  packed lowercased UTF-8 path bytes (used for matching).
///   Orig-path blob:   packed original-case UTF-8 path bytes (used for display).
///
/// Two blobs keep matching case-insensitive while preserving the real casing in
/// results. After `mmap` the mapped region is bound straight to typed pointers
/// with zero copying.
public enum IndexFormat {
    /// "MHFINDC1" as a little-endian UInt64 tag.
    public static let magic: UInt64 = 0x3143_444E_4946_484D
    public static let version: UInt32 = 2
    public static let headerSize = 40

    @inline(__always)
    static func align8(_ n: Int) -> Int { (n + 7) & ~7 }
}

/// A single logical record, used only during building. At query time we work off
/// the raw parallel pointers instead.
public struct IndexEntry {
    public var pathLower: [UInt8]   // lowercased path bytes (for matching)
    public var pathOrig: [UInt8]    // original-case path bytes (for display)
    public var basenameStart: Int   // index of basename within pathLower
    public var extID: UInt16
    public var isDir: Bool

    public init(pathLower: [UInt8], pathOrig: [UInt8], basenameStart: Int, extID: UInt16, isDir: Bool) {
        self.pathLower = pathLower
        self.pathOrig = pathOrig
        self.basenameStart = basenameStart
        self.extID = extID
        self.isDir = isDir
    }
}

/// Compute the word-boundary bitmap for a basename slice.
/// A boundary is a byte that starts a new "word": position 0, any byte after a
/// separator (`/ . - _ space`), or a digit run start. Only the first 64 basename
/// bytes are tracked (one bit each).
@inline(__always)
func computeBoundaries(_ bytes: ArraySlice<UInt8>) -> UInt64 {
    var mask: UInt64 = 0
    var prev: UInt8 = 0x2F // pretend previous was a separator so index 0 is a boundary
    var i = 0
    for b in bytes {
        if i >= 64 { break }
        let sep = (prev == 0x2F || prev == 0x2E || prev == 0x2D || prev == 0x5F || prev == 0x20)
        let digitStart = (b >= 0x30 && b <= 0x39) && !(prev >= 0x30 && prev <= 0x39)
        if sep || digitStart {
            mask |= (1 << UInt64(i))
        }
        prev = b
        i += 1
    }
    return mask
}

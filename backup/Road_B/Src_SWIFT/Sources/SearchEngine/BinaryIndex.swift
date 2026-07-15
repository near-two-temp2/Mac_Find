import Foundation

/// On-disk / in-memory binary index (mmap-friendly, parallel-array layout).
///
/// Layout, all little-endian:
///
///   Header (32 bytes):
///     magic:        UInt64  = 0x42_444E_49_48_46_4D2 masked to "MHFINDB1" style tag
///     version:      UInt32
///     entryCount:   UInt32
///     bytesCount:   UInt64  (size of the packed path blob)
///     reserved:     UInt64
///
///   Parallel arrays (entryCount elements each, in this order):
///     masks[i]:        UInt64  path letter bitmask
///     bnMasks[i]:      UInt64  basename letter bitmask
///     bnBoundaries[i]: UInt64  word-boundary bitmap within the basename
///     byteOffsets[i]:  UInt32  offset into the path blob
///     byteLengths[i]:  UInt16  path byte length
///     bnStarts[i]:     UInt16  basename start offset within the path
///     extIDs[i]:       UInt16  interned extension id (0 = none)
///     flags[i]:        UInt8   bit0 = isDir
///
///   Path blob:
///     packed lowercased UTF-8 path bytes (no separators)
///
/// The whole file is designed so that after `mmap` we can bind the mapped
/// region straight to typed pointers with zero copying.
public enum IndexFormat {
    public static let magic: UInt64 = 0x3142_444E_4946_484D // "MHFINDB1" LE-ish tag
    public static let version: UInt32 = 1
    public static let headerSize = 32
}

/// A single logical record, used only during building. At query time we work
/// off the raw parallel pointers instead.
public struct IndexEntry {
    public var pathLower: [UInt8]   // lowercased path bytes
    public var basenameStart: Int   // index of basename within pathLower
    public var extID: UInt16
    public var isDir: Bool

    public init(pathLower: [UInt8], basenameStart: Int, extID: UInt16, isDir: Bool) {
        self.pathLower = pathLower
        self.basenameStart = basenameStart
        self.extID = extID
        self.isDir = isDir
    }
}

/// Compute the word-boundary bitmap for a basename slice.
/// A boundary is a byte that starts a new "word": position 0, any byte after a
/// separator (`/ . - _ space`), or a lowercase→uppercase camelCase transition.
/// Only the first 64 basename bytes are tracked (one bit each).
@inline(__always)
func computeBoundaries(_ bytes: ArraySlice<UInt8>) -> UInt64 {
    var mask: UInt64 = 0
    var prev: UInt8 = 0x2F // pretend previous was a separator so index 0 is a boundary
    var i = 0
    for b in bytes {
        if i >= 64 { break }
        let sep = (prev == 0x2F || prev == 0x2E || prev == 0x2D || prev == 0x5F || prev == 0x20)
        // camelCase transition uses the *original* case; callers pass lowercased
        // bytes so this mainly catches separators + digit runs.
        let digitStart = (b >= 0x30 && b <= 0x39) && !(prev >= 0x30 && prev <= 0x39)
        if sep || digitStart {
            mask |= (1 << UInt64(i))
        }
        prev = b
        i += 1
    }
    return mask
}

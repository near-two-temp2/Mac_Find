import Foundation

/// 64-bit letter/character bitmask used for O(1) candidate pre-filtering.
///
/// Bit layout (mirrors Cling's scheme, see open-source-analysis.md §3.3):
///   Bits 0-25:  letters a-z
///   Bits 26-35: digits 0-9
///   Bit 36:     '.' (dot)
///   Bit 37:     '-' (hyphen)
///   Bit 38:     '_' (underscore)
/// Every other byte is ignored (contributes no bit), which is fine: the mask
/// is only ever used as a *necessary* condition, never a sufficient one.
public enum Bitmask {

    /// Set the bit for a single (already lowercased) byte, or leave the mask
    /// untouched if the byte isn't one of the tracked characters.
    @inline(__always)
    public static func addByte(_ byte: UInt8, to mask: inout UInt64) {
        // a-z
        if byte >= 0x61, byte <= 0x7A {
            mask |= (1 << UInt64(byte - 0x61))
            return
        }
        // 0-9
        if byte >= 0x30, byte <= 0x39 {
            mask |= (1 << UInt64(26 + (byte - 0x30)))
            return
        }
        switch byte {
        case 0x2E: mask |= (1 << 36) // '.'
        case 0x2D: mask |= (1 << 37) // '-'
        case 0x5F: mask |= (1 << 38) // '_'
        default: break
        }
    }

    /// Compute the combined mask over a byte buffer (bytes must be lowercased).
    @inline(__always)
    public static func compute(_ bytes: UnsafeBufferPointer<UInt8>) -> UInt64 {
        var mask: UInt64 = 0
        for b in bytes { addByte(b, to: &mask) }
        return mask
    }

    /// Convenience for an array slice.
    public static func compute(_ bytes: [UInt8]) -> UInt64 {
        bytes.withUnsafeBufferPointer { compute($0) }
    }

    /// The core prefilter test: can `entryMask` possibly contain everything in
    /// `queryMask`? If any required bit is missing, the entry is impossible.
    @inline(__always)
    public static func contains(entry entryMask: UInt64, query queryMask: UInt64) -> Bool {
        (entryMask & queryMask) == queryMask
    }
}

/// ASCII-lowercase a byte in place (leaves non A-Z untouched, including UTF-8
/// continuation bytes which we deliberately do not case-fold).
@inline(__always)
public func asciiLower(_ byte: UInt8) -> UInt8 {
    (byte >= 0x41 && byte <= 0x5A) ? byte + 0x20 : byte
}

import Foundation

/// Interns file extensions into small integer IDs so Phase-1 filtering can do a
/// single UInt16 compare instead of string work. ID 0 is reserved for "no
/// extension". The table is rebuilt fresh per index build; IDs are not stable
/// across builds, which is fine because it's only consulted within one session.
public final class ExtensionTable {
    private var map: [String: UInt16] = [:]
    private(set) public var names: [String] = [""] // index 0 => ""

    public init() {}

    /// Intern the (lowercased) extension of a basename, returns its ID.
    public func intern(basenameLower: ArraySlice<UInt8>) -> UInt16 {
        guard let ext = Self.extractExtension(basenameLower) else { return 0 }
        if let id = map[ext] { return id }
        let id = UInt16(truncatingIfNeeded: names.count)
        map[ext] = id
        names.append(ext)
        return id
    }

    /// Look up an existing ID for a query extension without inserting.
    public func lookup(_ ext: String) -> UInt16? { map[ext.lowercased()] }

    /// Extract the extension (bytes after the last dot that isn't a leading dot).
    static func extractExtension(_ basename: ArraySlice<UInt8>) -> String? {
        guard let dot = basename.lastIndex(of: 0x2E) else { return nil }
        // Leading-dot files (".gitignore") have no extension.
        if dot == basename.startIndex { return nil }
        let extBytes = basename[basename.index(after: dot)...]
        if extBytes.isEmpty || extBytes.count > 16 { return nil }
        return String(decoding: extBytes, as: UTF8.self)
    }
}

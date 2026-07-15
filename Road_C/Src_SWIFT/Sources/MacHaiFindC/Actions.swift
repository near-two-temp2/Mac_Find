import Foundation
import AppKit

/// AppKit-backed side effects invoked from the GUI (reveal / open / copy).
/// Isolated here so the views stay declarative and the model stays testable.
enum Actions {

    static func revealInFinder(path: String) {
        let url = URL(fileURLWithPath: path)
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }

    static func open(path: String) {
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    static func copyToPasteboard(_ string: String) {
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(string, forType: .string)
    }

    /// A small icon for a path, using the system's file-type icons.
    static func icon(for path: String, isDir: Bool) -> NSImage {
        NSWorkspace.shared.icon(forFile: path)
    }
}

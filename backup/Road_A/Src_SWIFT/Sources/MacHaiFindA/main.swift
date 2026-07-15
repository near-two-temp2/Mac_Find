import Foundation
import SearchFSKit

// Entry point for Road_A / Swift.
//
// Default (no args, or launched as a .app bundle): start the SwiftUI GUI.
// With CLI args: run a headless search — used for CI smoke tests where no
// window server is available.
//
//   MacHaiFindA <term> [--files-only|--dirs-only] [--exact] [--case-sensitive]
//                      [--limit N] [--volume PATH]... [--self-test]
//
// Using an explicit main.swift (rather than @main) keeps the branch trivial and
// avoids pulling AppKit/SwiftUI into a headless CI run until we actually need it.

let args = Array(CommandLine.arguments.dropFirst())

if args.isEmpty {
    // GUI mode. Defined in GUIApp.swift; guarded so headless CLI never touches it.
    runGUI()
} else {
    exit(CLI.run(args))
}

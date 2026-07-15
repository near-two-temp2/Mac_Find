import Foundation

// Entry point for Road_C / Swift — the hybrid flagship.
//
// Default (no args, or launched as a .app bundle): start the SwiftUI GUI.
// With CLI args: run headless subcommands — used for CI smoke tests where no
// window server is available.
//
//   machaifind-c gui                     explicitly launch the GUI
//   machaifind-c index [--root PATH]...  build the binary index
//   machaifind-c search <term> [opts]    query (index if present, else searchfs)
//   machaifind-c --self-test             build a tiny index in a temp dir and
//                                        assert search works end-to-end (CI)
//
// Using an explicit main.swift (rather than @main) keeps the branch trivial and
// avoids pulling AppKit/SwiftUI into a headless CI run until we actually need it.

let args = Array(CommandLine.arguments.dropFirst())

if args.isEmpty || args.first == "gui" {
    runGUI()
} else {
    exit(CLI.run(args))
}

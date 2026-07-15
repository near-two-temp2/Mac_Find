import Foundation
import SearchEngine

// Entry point. Dispatches between CLI subcommands (for CI smoke tests) and the
// SwiftUI GUI. Using an explicit main.swift (not @main) keeps the CLI path from
// requiring an app bundle / window server, which matters on headless CI.
//
//   machaifind-b index [--root PATH] [--out PATH] [--hidden]
//   machaifind-b search <query> [--index PATH] [--limit N] [--files|--dirs] [--ext EXT]
//   machaifind-b gui        (default when no args)

let args = Array(CommandLine.arguments.dropFirst())

switch args.first {
case "index":
    exit(CLI.runIndex(Array(args.dropFirst())))
case "search":
    exit(CLI.runSearch(Array(args.dropFirst())))
case "gui", nil, "":
    GUIApp.run()
case "--help", "-h", "help":
    CLI.printUsage()
    exit(0)
default:
    FileHandle.standardError.write(Data("unknown command: \(args[0])\n".utf8))
    CLI.printUsage()
    exit(2)
}

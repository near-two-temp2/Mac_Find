// swift-tools-version:5.7
import PackageDescription

// Road_C / Swift — the hybrid flagship. SwiftUI GUI over a hybrid search engine:
//   primary = self-built mmap binary index (parallel bitmask prefilter + fzf)
//   fallback = searchfs() catalog scan when the index is missing/corrupt
//
// Structure:
//   CSearchFS     —— C shim exposing searchfs()/fsgetpath() (the fallback engine)
//   HybridEngine  —— pure-logic Swift engine: index build/mmap/search, fzf,
//                    searchfs wrapper, FSEvents watcher. UI-free so it compiles
//                    and self-tests headlessly on any macOS runner.
//   MacHaiFindC   —— executable: SwiftUI GUI by default; `index`/`search`/
//                    `--self-test` CLI subcommands for CI smoke tests.
let package = Package(
    name: "MacHaiFindC",
    platforms: [
        // Dev machine runs macOS 12.7.6 (Intel); keep the minimum at 12 so the
        // shipped .app opens there. All SwiftUI/AppKit APIs used by the GUI
        // (foregroundStyle(.secondary/.tertiary), background(_:in:), Capsule,
        // List(selection:), onChange(of:){v in}) are available on macOS 12.
        .macOS(.v12)
    ],
    products: [
        .executable(name: "machaifind-c", targets: ["MacHaiFindC"])
    ],
    targets: [
        .target(
            name: "CSearchFS",
            path: "Sources/CSearchFS"
        ),
        .target(
            name: "HybridEngine",
            dependencies: ["CSearchFS"],
            path: "Sources/HybridEngine"
        ),
        .executableTarget(
            name: "MacHaiFindC",
            dependencies: ["HybridEngine"],
            path: "Sources/MacHaiFindC",
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("SwiftUI"),
                .linkedFramework("Combine"),
                .linkedFramework("Carbon"),
                .linkedFramework("CoreServices")
            ]
        ),
        .testTarget(
            name: "HybridEngineTests",
            dependencies: ["HybridEngine"],
            path: "Tests/HybridEngineTests"
        )
    ]
)

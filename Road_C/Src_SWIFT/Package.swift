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
        .macOS(.v13)
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

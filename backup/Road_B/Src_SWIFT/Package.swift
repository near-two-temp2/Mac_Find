// swift-tools-version:5.7
import PackageDescription

let package = Package(
    name: "MacHaiFindB",
    platforms: [
        .macOS(.v12)
    ],
    products: [
        // Single executable that behaves as:
        //   - GUI app when launched with no args (or `gui`)
        //   - `index` / `search` CLI subcommands for CI smoke tests
        .executable(name: "machaifind-b", targets: ["MacHaiFindB"]),
    ],
    targets: [
        // Pure-logic engine: index build, mmap load, bitmask prefilter, fzf scoring.
        // Kept UI-free so it compiles/tests headlessly on any runner.
        .target(
            name: "SearchEngine",
            path: "Sources/SearchEngine"
        ),
        .executableTarget(
            name: "MacHaiFindB",
            dependencies: ["SearchEngine"],
            path: "Sources/MacHaiFindB"
        ),
        .testTarget(
            name: "SearchEngineTests",
            dependencies: ["SearchEngine"],
            path: "Tests/SearchEngineTests"
        ),
    ]
)

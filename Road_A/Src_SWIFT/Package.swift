// swift-tools-version:5.7
import PackageDescription

// Road_A / Swift — searchfs() 无索引实时文件名搜索 GUI（SwiftUI）
//
// 结构：
//   CSearchFS       —— C shim，暴露 searchfs()/fsgetpath()/getattrlist() 给 Swift
//   SearchFSKit     —— Swift 搜索引擎（封装 searchfs 调用序列、EBUSY 重试、双卷、路径还原、过滤）
//   MacHaiFindA     —— 可执行 target：默认启动 SwiftUI GUI；带参数时走 CLI 冒烟模式
let package = Package(
    name: "MacHaiFindA",
    platforms: [
        // macOS 12: gives us SwiftUI Table, onSubmit and .number formatting.
        // CI runs on macos-latest (14+), so this is comfortably satisfied.
        .macOS(.v12)
    ],
    products: [
        .executable(name: "MacHaiFindA", targets: ["MacHaiFindA"])
    ],
    targets: [
        .target(
            name: "CSearchFS",
            path: "Sources/CSearchFS"
        ),
        .target(
            name: "SearchFSKit",
            dependencies: ["CSearchFS"],
            path: "Sources/SearchFSKit"
        ),
        .executableTarget(
            name: "MacHaiFindA",
            dependencies: ["SearchFSKit"],
            path: "Sources/MacHaiFindA"
        )
    ]
)

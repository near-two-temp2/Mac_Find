# Road_C · Swift — 混合引擎旗舰 (SwiftUI GUI)

对标 Windows Everything 的 macOS 秒速文件搜索，**Road_C 完整混合版**：
原生 SwiftUI GUI + 自建 mmap 二进制索引（主）+ `searchfs()` 实时兜底（备）。
本目录是「18 实现矩阵」中 Swift 语言的旗舰实现。

## 架构

```
┌──────────────────────── SwiftUI GUI (MacHaiFindC) ─────────────────────────┐
│ 菜单栏图标 + 全局快捷键(⌥⌘Space) + 搜索框 + 结果列表 + 在 Finder 中显示      │
│ AppModel(@MainActor ObservableObject): 60ms 去抖 + 后台搜索 + 索引生命周期  │
└────────────────────────────────────────────────────────────────────────────┘
                                    │
┌──────────────────── HybridEngine (纯逻辑, UI-free) ────────────────────────┐
│ 优先: mmap 二进制索引                     兜底: searchfs() 目录搜索          │
│  ┌──────────────────────────────────┐    ┌──────────────────────────────┐  │
│  │ IndexBuilder  遍历→并行数组序列化 │    │ SearchFSFallback             │  │
│  │ IndexSearcher mmap + 两阶段搜索:  │    │  经 CSearchFS 调 searchfs()  │  │
│  │   Phase1 concurrentPerform 并行   │    │  索引缺失/损坏/强制时启用     │  │
│  │          bitmask+扩展名+类型预过滤│    │  保证 100% 新鲜(较慢)          │  │
│  │   Phase2 FuzzyScore fzf 评分排序  │    └──────────────────────────────┘  │
│  │ FSEventsWatcher 文件变化→标记 stale│                                       │
│  └──────────────────────────────────┘                                       │
└────────────────────────────────────────────────────────────────────────────┘
                                    │
                    CSearchFS (C shim: searchfs()/fsgetpath())
```

参考 `../../../open-source-analysis.md` §3（Cling 索引）与 §5.4（推荐混合架构），
searchfs 调用序列改编自 `../../../Open_Ref/searchfs/main.m`（BSD-3）。

## 构建方式

权威构建在 **GitHub Actions 的 `macos-latest` runner**（见 `../../.github/workflows/build-c-swift.yml`）。
开发机为 macOS 12，本地 SwiftPM 因 CommandLineTools 无法解析 `PackageDescription` 而不能整包编译，
但各 target 已用 `swiftc` 单独通过类型检查与功能自测（详见下方「本地验证」）。

CI 上：

```bash
swift build -c release                    # 编译引擎 + GUI 可执行文件
swift test  -c release                    # 无头引擎单元测试(HybridEngineTests)
swift run   -c release machaifind-c --self-test   # 端到端流水线自测(CI gate)
# 之后 workflow 把裸可执行文件包成 MacHaiFindC.app 并上传
```

CLI 入口（同一可执行文件，无参启动 GUI）：

```bash
machaifind-c                              # 启动 SwiftUI GUI（默认）
machaifind-c index [--root PATH]...       # 建立二进制索引（默认 $HOME）
machaifind-c search <term> [--files-only|--dirs-only] [--limit N] [--fallback]
machaifind-c --self-test                  # CI 自测
```

## 已实现

- **原生 SwiftUI GUI**：搜索框、结果列表（系统图标 + basename + 灰显父路径 + 分数）、
  文件/目录/强制 searchfs 过滤开关、后端徽章（index/searchfs 变色）、状态栏耗时。
- **菜单栏 status item** + **全局快捷键 ⌥⌘Space**（Carbon `RegisterEventHotKey`）唤起/隐藏窗口。
- **在 Finder 中显示 / 打开 / 拷贝路径**（右键菜单 + 双击打开 + 底栏 Reveal 按钮）。
- **自建二进制索引**（`MHFINDC1` v2 格式）：并行数组 + 8 字节对齐 + mmap 零拷贝加载；
  同时存小写字节（匹配）与原始大小写字节（展示）两份路径 blob。
- **两阶段搜索**：Phase 1 `DispatchQueue.concurrentPerform` 分片并行 bitmask/扩展名/类型预过滤；
  Phase 2 fzf 评分（多锚点 + 边界奖励 + 连续奖励 + 间隙惩罚）。
- **扩展名约束**：`.pdf` / `*.swift` 形式的查询走 UInt16 扩展名 ID 快速过滤。
- **路径查询**：含 `/` 的查询对整条路径做模糊匹配。
- **searchfs() 兜底**：索引缺失/损坏/UI 勾选「强制」时经 `CSearchFS` 调内核 catalog 搜索，
  双卷（`/` 与 `/System/Volumes/Data`）+ EBUSY 重试。
- **FSEvents 监听**：文件变化去抖后把索引标记为 stale。
- **无头自测**：`--self-test` 与 `HybridEngineTests` 覆盖 建索引→mmap→预过滤→fzf 全链路。

## TODO

- **FSEvents 增量原地更新**：目前仅标记 stale 并触发整体重建；未实现 Cling 式变更日志与
  索引原地追加/删除（需要可压缩写入器）。
- **searchfs() 初始扫描**：初始建索引仍用 `FileManager.enumerator`；分析报告建议改用
  `searchfs()` 以更快更完整地完成首扫。
- **多卷/多作用域索引**：当前默认只索引 `$HOME`（其余靠 searchfs 兜底）；未做 Cling 式
  home/library/applications/system 分作用域独立 `.idx`。
- **SIMD 加速**：fzf 锚点搜索为标量循环，未做 `SIMD16<UInt8>` 字节并行。
- **结果富化**：未展示大小/修改时间/UTI 等列；未做 Quick Look / 拖放 / 权限提升。
- **键盘上下选中导航**：依赖 List 默认选择，未加自定义 ↑/↓ + Enter 焦点流。
- **代码签名/公证**：CI 仅 ad-hoc 签名，产物为未公证的 CI artifact。

## CI Artifact

- Workflow：`../../.github/workflows/build-c-swift.yml`（`build-c-swift`）
- Artifact 名：**`road-c-swift-app`**（内含 `MacHaiFindC.app`）

## 本地验证（在 macOS 12 开发机上，绕过 SwiftPM 整包编译）

```bash
# 核心引擎功能自测（真实建索引→mmap→fzf，全部断言通过）
swiftc -O -o /tmp/smoke Sources/HybridEngine/{Bitmask,ExtensionTable,BinaryIndex,\
FuzzyScore,IndexBuilder,IndexSearcher}.swift <driver-main.swift> && /tmp/smoke

# C shim 编译（-Wall -Wextra 干净）
cc -c -Wall -Wextra -I Sources/CSearchFS/include Sources/CSearchFS/csearchfs.c

# 引擎/GUI 类型检查（需为 CSearchFS 造临时 modulemap 后）
swiftc -emit-module -I <modmap> Sources/HybridEngine/*.swift
swiftc -typecheck  -I <engine-module> -I <modmap> Sources/MacHaiFindC/*.swift
```

以上四项本地均通过（引擎类型检查、模块 emit、GUI 类型检查、C 编译、5 项功能断言）。

## 目录结构

```
Road_C/Src_SWIFT/
├── Package.swift                         SwiftPM 清单（3 target + 测试）
├── README.md
├── Sources/
│   ├── CSearchFS/                        C shim（searchfs 兜底引擎）
│   │   ├── include/csearchfs.h
│   │   └── csearchfs.c
│   ├── HybridEngine/                     纯逻辑混合引擎（UI-free，可无头自测）
│   │   ├── Bitmask.swift                 64-bit 字符位掩码预过滤
│   │   ├── ExtensionTable.swift          扩展名→UInt16 ID interning
│   │   ├── BinaryIndex.swift             索引格式定义 + 词边界位图
│   │   ├── FuzzyScore.swift              fzf 评分算法
│   │   ├── IndexBuilder.swift            遍历文件系统→序列化二进制索引
│   │   ├── IndexSearcher.swift           mmap 加载 + 两阶段并行搜索
│   │   ├── SearchFSFallback.swift        searchfs() 兜底封装
│   │   ├── FSEventsWatcher.swift         FSEvents 变化监听
│   │   └── HybridEngine.swift            混合协调器（索引为主 + 兜底）
│   └── MacHaiFindC/                      可执行 target（GUI + CLI）
│       ├── main.swift                    入口：无参→GUI，有参→CLI
│       ├── CLI.swift                     index/search/--self-test 子命令
│       ├── AppModel.swift                @MainActor 视图模型
│       ├── Actions.swift                 Finder reveal / open / 拷贝
│       ├── GlobalHotkey.swift            Carbon 全局快捷键
│       ├── GUIApp.swift                  NSApplication + 菜单栏 + 窗口
│       └── ContentView.swift            SwiftUI 主界面
└── Tests/HybridEngineTests/              无头单元测试
    └── HybridEngineTests.swift
```

# Road_A · Swift — searchfs() 无索引实时文件搜索 GUI

macOS 原生 SwiftUI 桌面 app。搜索框 + 结果列表，后台引擎通过 C 互操作直接调用内核
`searchfs()` 系统调用**实时**搜索文件名——不建索引、每次扫描 APFS/HFS+ 的 B-Tree catalog，
比 `find` 快约 100 倍。

对标 Windows Everything 的「秒速搜索」，是 18 实现矩阵中 **路线 A（无索引实时引擎）** 的 Swift 版。

## 架构

```
Package.swift
Sources/
├── CSearchFS/                  C shim（把 searchfs 调用序列留在 C，最贴近系统头文件）
│   ├── include/csearchfs.h     模块头 + CSFS_* 选项枚举
│   └── csearchfs.c             searchfs() 驱动循环（改编自 Open_Ref/searchfs/main.m, BSD-3）
├── SearchFSKit/                Swift 搜索引擎
│   └── SearchEngine.swift      封装 C 层：双卷、选项、路径还原后的精细过滤、limit/取消
└── MacHaiFindA/                可执行 target
    ├── main.swift              入口：无参启 GUI；带参走 CLI（CI 冒烟）
    ├── GUIApp.swift            NSApplication + NSWindow 承载 SwiftUI（SwiftPM 可执行无 Info.plist，故手动建窗）
    ├── ContentView.swift       主界面：搜索框 + 选项栏 + 结果列表 + 状态栏
    ├── SearchViewModel.swift   防抖(200ms)、后台线程跑 searchfs、代际令牌丢弃过期结果
    └── CLI.swift               headless 搜索 + --self-test 自检
```

### 为什么把 searchfs 调用留在 C

`struct fssearchblock`、返回缓冲区的 `ATTR_*` 打包布局、`fsgetpath()` 的 objid 重组，
用 C 对着系统头文件写最稳（直接借鉴参考实现 `Open_Ref/searchfs/main.m`）。C 层 `csfs_search()`
跑完整搜索循环，每命中一个可还原路径就回调一次 Swift，Swift 侧负责收集/过滤/limit/取消。

### 关键技术点（对应路线 A 规格）

- **`fssearchblock` / `SRCHFS_*` 标志**：`csfs_search()` 内按 `CSFS_*` 选项翻译成
  `SRCHFS_MATCHFILES / MATCHDIRS / MATCHPARTIALNAMES / SKIPPACKAGES / SKIPINVISIBLE`。
- **`fsgetpath()` 路径还原**：由 `fsid + objid` 还原绝对路径；对象在命中与还原之间被删则静默跳过。
- **EBUSY 重试**：catalog 在搜索途中变更返回 EBUSY，`goto catalog_changed` 重试最多 5 次。
- **双卷**：Catalina+ 默认扫 `/` 与 `/System/Volumes/Data`（`csfs_data_volume_available()` 探测）。
- **大小写 / 子串 / 仅文件仅目录 / 结果上限**：内核先做不区分大小写的 name 子串匹配，
  Swift 侧 `accept()` 再按 basename 精确复核大小写与子串/精确，`limit` 命中即令 C 回调返回 0 提前停止。

## 已实现

- [x] SwiftUI GUI：搜索框、`文件+目录 / 仅文件 / 仅目录` 分段选择、`子串` / `区分大小写` 开关、结果上限输入框。
- [x] 结果列表：图标 + 文件名 + 灰色全路径；右键「在访达中显示 / 拷贝路径」；双击在访达显示。
- [x] 输入防抖 200ms，搜索在后台线程执行，代际令牌保证只展示最新一次结果；状态栏显示命中数与耗时。
- [x] 后台引擎：`searchfs()` 实时搜索、`fsgetpath()` 还原、EBUSY 重试、双卷、选项全通。
- [x] CLI 入口（`main.swift` 带参）+ `--self-test` 自检，供 CI 冒烟。
- [x] 原生菜单栏（Cmd-Q / Cmd-W / Cmd-M）。

## 构建

**权威构建 = GitHub Actions `macos-latest`**（见下）。本地开发机为 macOS 12 且 SwiftPM
manifest 工具链不全，不作要求。

```bash
# 在 macos-latest / 完整 Xcode 环境：
cd Road_A/Src_SWIFT
swift build -c release
.build/release/MacHaiFindA                       # 启动 GUI
.build/release/MacHaiFindA "Package.swift" --files-only --limit 20   # CLI 搜索
.build/release/MacHaiFindA --self-test           # CI 冒烟自检
```

> 注：SwiftPM 可执行产物本身没有 Info.plist，直接跑也能弹出窗口（`GUIApp.swift` 里手动
> `setActivationPolicy(.regular)`）。CI 会把二进制包进 `MacHaiFindA.app` bundle，作为原生 app 分发。

### CLI 选项

```
MacHaiFindA <term> [--files-only|--dirs-only] [--exact] [--case-sensitive]
                   [--limit N] [--volume PATH]... [--self-test] [-h]
```

## CI / Artifact

- Workflow：`.github/workflows/build-a-swift.yml`（`on: [push, workflow_dispatch]`，
  `paths:` 仅匹配 `Road_A/Src_SWIFT/**`，`runs-on: macos-latest`）。
- 步骤：`swift build -c release` → 跑 `--self-test` 冒烟 → 打包 `MacHaiFindA.app` → 上传。
- **Artifact 名：`road-a-swift-app`**（内含 `MacHaiFindA.app`）。

## TODO

- [ ] `.app` 未做代码签名 / notarization；首次运行需右键「打开」绕过 Gatekeeper。
- [ ] `--skip-packages` / `--skip-invisibles`（C 层已支持 `CSFS_SKIP_*`）尚未接到 GUI 开关。
- [ ] 结果排序目前按 searchfs 返回顺序；可加按路径深度 / basename 排序。
- [ ] 非默认卷选择的 GUI（当前 GUI 固定扫默认双卷，CLI 可 `--volume` 指定）。
- [ ] 应用图标（`.icns`）与 Retina 资源。
- [ ] 增量刷新（FSEvents）不属路线 A 范畴（属路线 B/C），此实现为纯实时无索引。

## 参考

- `../../Open_Ref/searchfs/main.m` —— searchfs() 调用序列与打包结构原型（BSD-3-Clause）。
- `man 2 searchfs`、`man 2 getattrlist`、`<sys/fsgetpath.h>`。

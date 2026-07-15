# Road_C · Go — Mac Find (混合完整版 GUI)

macOS 极速文件搜索的 **Go / Fyne** 实现，路线 C（完整混合引擎）。
搜索框 + 结果列表 + 「在 Finder 中显示」的桌面 app；后台是**混合引擎**：
主用自建二进制索引（goroutine 并行 bitmask 预过滤 + fzf 模糊评分），
索引缺失/损坏时**自动降级**到 cgo 调用 `searchfs(2)` 的实时兜底扫描。

## 架构

```
cmd/macfind/            单一二进制，GUI 默认 + CLI 子命令
├── main.go             入口：有 CLI 子命令走 CLI，否则启动 GUI
├── gui.go              Fyne 窗口：搜索框 / 结果列表 / Show in Finder / Build Index
└── cli.go              index / search / selftest 子命令（脚本与 CI 冒烟）

internal/
├── bitmask/            Cling 式 64-bit 字符位掩码，O(1) 候选预过滤
├── fuzzy/              fzf 风格模糊评分（锚点枚举 + 边界/连续加成）
├── index/              自建二进制索引：build.go 建索引 / search.go 两阶段并行查询
├── searchfs/           cgo 封装 searchfs(2) 内核 catalog 搜索（兜底引擎）
│   ├── searchfs_darwin.go   真实实现（macOS）
│   └── searchfs_other.go    非 macOS 空实现（仅供本地工具链编译）
└── engine/             混合编排：索引优先，缺失/损坏时降级 searchfs()
```

### 混合搜索流程（对应 open-source-analysis.md §5.4）

1. `engine.New(indexPath)` 尝试加载 `~/Library/Caches/macfind-roadc-go/index.idx`。
2. 有索引：`index.Search()`
   - **Phase 1**：按 CPU 核数分片，goroutine 并行做 bitmask 预过滤（`entryMask & queryMask == queryMask`）。
   - **Phase 2**：对存活候选做 fzf 评分，basename 命中额外加分，按分数排序。
3. 无索引 / 索引损坏（magic 不符、大小不匹配 → `ErrBadIndex`）：降级到
   `searchfs.Search()`，cgo 直接调内核 catalog 搜索，双卷（`/` 与 `/System/Volumes/Data`）。

### 二进制索引格式（`internal/index/format.go`）

并行数组 + mmap 友好布局（Go 无 stdlib mmap，用整文件 `[]byte` + 零拷贝子切片达到同等随机访问）：

```
Header(32B): magic "MCFIDX01" | version | entryCount | bytesLen | reserved
Arrays[N]:   masks u64 | bnMasks u64 | offset u32 | length u32 | bnStart u32 | isDir u8
Blob:        拼接的小写 UTF-8 路径字节
```

写索引采用「临时文件 + rename」原子落盘，避免半截索引被误当有效。

## 构建方式

**权威构建 = GitHub Actions（macos-latest）**，见 `../../.github/workflows/build-c-go.yml`。
Fyne v2.6.3 需要 `CGO_ENABLED=1`（OpenGL / cgo）；searchfs 兜底也需要 cgo。
后台 goroutine 更新 UI 走 `fyne.Do(func(){...})`（v2.6+ 引入；v2.5.x 无此 API）。

本地（macOS）：

```bash
cd Road_C/Src_GO
go mod download

# CLI 冒烟（无需 GUI / 无需 root）
CGO_ENABLED=1 go run ./cmd/macfind selftest

# 打包 .app（CI 用 fyne package；v2.6 flag 为 -appID / -sourceDir）
go install fyne.io/fyne/v2/cmd/fyne@v2.6.3
CGO_ENABLED=1 fyne package -os darwin -name MacFindRoadCGo \
  -appID com.macfind.roadc.go -sourceDir ./cmd/macfind
```

### CLI 子命令

```bash
macfind                 # 无参数 → 启动 GUI
macfind index [root...] # 建索引（默认 $HOME + /Applications）
macfind search <query>  # 走混合引擎查询，打印排序结果
macfind selftest        # CI 冒烟：临时索引 + 查询自检，退出码即结果
```

## 已实现

- Fyne GUI：搜索框（逐键去抖实时搜索）、结果列表（文件/目录图标）、
  「Show in Finder」（`open -R`）、「Build / Rebuild Index」（后台建索引 + 热重载）、
  状态栏显示命中数 / 当前引擎（index vs searchfs fallback）/ 耗时。
- 自建二进制索引：build（`filepath.WalkDir`，跳过不可读目录）、原子落盘、
  校验加载（magic + 大小），两阶段 goroutine 并行搜索。
- fzf 模糊评分：锚点枚举 + 贪婪匹配 + 词边界/连续加成/间隙惩罚，basename 优先。
- searchfs cgo 兜底：封装 `searchfs(2)` + `fsgetpath()`，双卷、EBUSY 重试、去重。
- 混合编排：索引优先，缺失/损坏 → searchfs 降级；引擎来源回传给 UI。
- 单元测试：bitmask / fuzzy / index 三包（`go test ./internal/...`），CI 会跑。

## TODO

- 初始扫描改用 `searchfs()` 代替 `WalkDir`（更快更完整，见 §5.4 改进点 1）。
- FSEvents 增量更新索引（当前需手动 Rebuild）。
- 扩展名 ID / basename bitmask 二级预过滤、SIMD 字节搜索加速 Phase 1。
- 索引真正 mmap（`golang.org/x/exp/mmap` 或 syscall.Mmap）降低大索引内存峰值。
- GUI：文件/目录过滤开关、大小写、结果上限、双击打开、右键菜单。
- searchfs 结果的 `isDir` 目前恒为 false（内核返回未区分），待补 `stat`。
- 索引新鲜度提示 / 后台定期重建。

## CI Artifact

- Workflow：`.github/workflows/build-c-go.yml`
- Artifact 名：**`road-c-go-app`**（含打包的 `.app` 及 CLI 二进制）

# Road_B · Go — 自建二进制索引 + fzf 模糊搜索（Fyne GUI）

macOS 极速文件搜索 18 实现矩阵中的 **路线 B / Go** 实现。

一个 [Fyne](https://fyne.io/) 桌面 GUI（搜索框 + 结果列表），后台引擎是**自建二进制索引 + bitmask 预过滤 + fzf 评分**，架构参考 Cling（见 `../../../open-source-analysis.md` §3）。

## 引擎设计

### 二进制索引（`internal/index/`）

并行数组布局，mmap 友好，参考 Cling 的 `.idx` 格式：

| 列 | 类型 | 含义 |
|----|------|------|
| `Masks[i]` | `uint64` | 整条路径的字母 bitmask（O(1) 预过滤） |
| `BNMasks[i]` | `uint64` | 仅 basename 的 bitmask |
| `ExtIDs[i]` | `uint32` | 内部化（interned）的小写扩展名 ID（0 = 无） |
| `ByteOffsets[i]` | `uint32` | 路径在 `AllBytes` 中的起始偏移 |
| `ByteLengths[i]` | `uint16` | 路径字节长度 |
| `BNStarts[i]` | `uint16` | basename 在路径中的起始位置 |
| `IsDirs[i]` | `uint8` | 是否目录 |
| `AllBytes` | `[]byte` | 打包的小写 UTF-8 路径字节 |

**bitmask 编码**（与 open-source-analysis.md §3.3 一致）：

```
Bits 0-25:  字母 a-z
Bits 26-35: 数字 0-9
Bit 36:     '.'    Bit 37: '-'    Bit 38: '_'
```

- 磁盘格式：24 字节头（magic `MACFINDB` + entryCount + bytesLen）→ 各列连续排布 → 扩展名表 → 打包路径字节。全部小端。
- 加载：`index.Open()` 用 `unix.Mmap`（`MAP_PRIVATE|PROT_READ`）映射文件（`internal/index/mmap_unix.go`）。整数列在解析时拷进 Go 切片（可移植、endian 安全），体量最大的 `AllBytes` 直接别名 mmap 区域。
- 构建：`index.Build()` 用 `filepath.WalkDir` 遍历，默认剪掉 `.git`/`node_modules`/`Caches` 等噪声目录（不跟随符号链接）。

### 两阶段搜索（`internal/search/`）

- **Phase 1（并行预过滤）**：按 CPU 核数把索引分片，多个 goroutine 并发做
  `Masks[i] & queryMask != queryMask` 的 bitmask 快速排除 + 扩展名/目录过滤。一次 `uint64` 比较即可淘汰绝大多数候选。
- **Phase 2（fzf 评分）**：对存活候选跑 fzf 风格模糊评分（`internal/search/fzf.go`）：多锚点枚举 + 贪婪正向匹配 + 词边界奖励 + 连续奖励 + 间隙惩罚（分值与 §3.4 对齐）。优先对 basename 评分，命中再加权。
- 结果按分数降序、路径更短、字典序排序后取前 N。

## 构建方式

**权威构建 = GitHub Actions `macos-latest` runner**（本地开发机为旧版 macOS，不在本地编译）。

CI workflow：`.github/workflows/build-b-go.yml`
- 触发：`push`（仅 `Road_B/Src_GO/**` 路径变更）+ `workflow_dispatch`。
- `runs-on: macos-latest`，`actions/setup-go`（Go 1.23），Fyne 需 `CGO_ENABLED=1`。
- 步骤：`go mod tidy` → `go vet` → `go test ./internal/...` → `go build` 出二进制 → CLI 冒烟测试（建索引 + 查询）→ `fyne package` 出 `.app`（失败则手工组装 `.app`）→ 上传 artifact。

**CI artifact 名：`road-b-go-app`**（含 `macfind.app` 与裸二进制 `macfind`）。

本地如需尝试（非必须、且需要磁盘空间与 Xcode CLT）：

```bash
cd Road_B/Src_GO
CGO_ENABLED=1 go build -o macfind ./cmd/macfind
```

## 用法

GUI（默认）：

```bash
./macfind                 # 启动 Fyne 窗口
```

点 **Build Index** 建索引（默认扫描 `$HOME` + `/Applications`，写入
`~/Library/Caches/com.macfind.roadb.go/index.idx`）；随后在搜索框输入即时搜索。下次启动会自动加载已有索引。

CLI（供脚本与 CI 冒烟）：

```bash
./macfind index  -o my.idx -root ~/Documents   # 建索引
./macfind search -i my.idx -n 30 report        # 查索引，fzf 排序
./macfind search -i my.idx -ext pdf report     # 限定扩展名
./macfind search -i my.idx -d config           # 仅目录
```

## 已实现

- [x] 自建二进制索引：并行数组 + `uint64` bitmask + basename bitmask + 扩展名 ID
- [x] mmap（`unix.Mmap`）加载索引；小端磁盘格式，带 magic 校验与截断检测
- [x] 文件系统遍历建索引（默认剪枝噪声目录、可跳过隐藏文件、不跟随符号链接）
- [x] 两阶段搜索：goroutine 并行 bitmask/扩展名预过滤 + fzf 评分排序
- [x] Fyne GUI：搜索框 + 结果列表 + Build Index 按钮 + 即时搜索 + 状态栏
- [x] `index` / `search` CLI 入口（CI 冒烟）
- [x] 引擎单元测试（`internal/search/search_test.go`：建索引/查询/bitmask 排除/扩展名过滤/save-load 往返）
- [x] CI workflow（macos-latest，产出 `.app` + 二进制）

## TODO

- [ ] **本地未编译验证**：开发机磁盘满（100%，Go 无法写构建缓存），本地未跑通编译/测试；
      依赖 CI 作为权威构建。Fyne 版本已从最初的 v2.5.3 上调到 **v2.6.3**，因为 `fyne.Do`
      （从后台 goroutine 安全更新 UI 的正确方式）是 v2.6 才引入的。
- [ ] FSEvents 增量更新（当前为全量重建；进阶项）
- [ ] `getattrlistbulk` / `fts` 加速遍历（当前用 `filepath.WalkDir`，够用但非最快）
- [ ] 保留原始大小写路径（当前索引只存小写字节，展示的是小写路径）
- [ ] 结果双击「在 Finder 中显示」/ 打开等操作
- [ ] SIMD 加速的字节查找（Go 暂用标量循环）
- [ ] 多卷 / 外置卷索引与 `/Volumes/*` 作用域
- [ ] 索引分片持久化与增量落盘（当前一次性全量 Save）

## 文件结构

```
Road_B/Src_GO/
├── go.mod                        # 模块定义（Fyne v2.6.3 + golang.org/x/sys）
├── Icon.png                      # 打包用占位图标（64×64）
├── cmd/macfind/
│   ├── main.go                   # 入口 + 子命令分发（gui / index / search）
│   ├── gui.go                    # Fyne GUI（搜索框 + 结果列表 + Build Index）
│   └── cli.go                    # index / search CLI（CI 冒烟）
└── internal/
    ├── index/
    │   ├── mask.go               # bitmask 编码
    │   ├── index.go              # 并行数组、二进制 save/load、Builder
    │   ├── walk.go               # 文件系统遍历建索引
    │   ├── location.go           # 默认索引路径与扫描根
    │   ├── mmap_unix.go          # darwin/linux mmap 加载
    │   └── mmap_other.go         # 其它平台回退（ReadFile）
    └── search/
        ├── fzf.go                # fzf 风格模糊评分
        ├── search.go             # 两阶段并行搜索
        └── search_test.go        # 引擎单元测试
```

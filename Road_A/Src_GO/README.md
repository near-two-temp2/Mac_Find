# Road A · Go — searchfs 无索引实时搜索 GUI

Mac_Find 实现矩阵中「路线 A × Go」的实现：一个 **Fyne** 桌面 GUI（搜索框 + 结果列表），
后台搜索引擎通过 **cgo** 直接调用 macOS 内核 `searchfs(2)` 系统调用，在 APFS/HFS+ 卷的
B-Tree catalog 上**实时**搜索文件名——**不建索引**，每次查询都是一次全卷 catalog 扫描，
比 `find` 快约 100 倍。

参考实现：`../../../Open_Ref/searchfs/main.m`（653 行 C，BSD-3）。本实现复用了其
`fssearchblock` 装配、`SRCHFS_*` 标志、结果 buffer 解包、`fsgetpath()` 路径还原、
EBUSY 重试与 Catalina+ 双卷（`/` 与 `/System/Volumes/Data`）策略。

## 构建方式

**权威构建走 GitHub Actions（`macos-latest`）**，本地无需成功编译。

CI：`.github/workflows/build-a-go.yml`（`build-a-go`）。
`paths` 只匹配 `Road_A/Src_GO/**`，`runs-on: macos-latest`，`CGO_ENABLED=1`。
流程：`setup-go@v5` → `go mod tidy` → `go vet` → `go build`（原始二进制）→ CLI 冒烟测试 →
`fyne package` 打 `.app` → `upload-artifact`。

本地手动构建（macOS，需 Xcode CLT）：

```bash
cd Road_A/Src_GO
CGO_ENABLED=1 go build -o macfind-a-go .   # 编译 GUI + cgo 引擎
./macfind-a-go                             # 启动 GUI
```

打包 `.app`：

```bash
go install fyne.io/fyne/v2/cmd/fyne@v2.5.2
fyne package --os darwin --name MacFindAGo --app-id org.macfind.roada.go --icon Icon.png
```

## CLI 冒烟入口

保留了一个 CLI 入口供 CI 冒烟与脚本使用（GUI 与 CLI 共用同一个 `engine` 包）：

```bash
./macfind-a-go --cli --limit 20 Applications        # 子串搜索，限 20 条
./macfind-a-go --cli --files-only --limit 10 bash   # 仅文件
./macfind-a-go --cli --dirs-only  --limit 10 Library # 仅目录
./macfind-a-go --cli --case-sensitive README        # 大小写敏感
```

## 已实现

- **cgo 封装 `searchfs(2)`**（`internal/engine/searchfs_darwin.go`）：装配 `fssearchblock`、
  `ATTR_CMN_NAME` 搜索属性、`ATTR_CMN_FSID | ATTR_CMN_OBJID` 返回属性，解包 packed 结果 buffer。
- **`fsgetpath()` 路径还原**：由 fsid + objid（`fid_objno | fid_generation<<32`）还原完整路径；
  还原失败（对象在匹配后被删除）时静默跳过。
- **仅文件 / 仅目录**：`SRCHFS_MATCHFILES` / `SRCHFS_MATCHDIRS` 组合；GUI 两个复选框互斥。
- **子串匹配**：`SRCHFS_MATCHPARTIALNAMES`（内核侧大小写不敏感子串）。
- **大小写敏感**：内核只做大小写不敏感匹配，故置位时在 basename 上做二次后过滤。
- **结果上限**：贯穿引擎与 GUI，达到上限即停。
- **EBUSY 重试**：catalog 变更时重启搜索，最多 5 次。
- **EAGAIN 续跑**：分批 `searchfs` 调用直到搜索完成。
- **Catalina+ 双卷**：默认搜索 `/` 与 `/System/Volumes/Data`，各卷结果按路径去重。
- **卷能力探测**：`getattrlist` 检查 `VOL_CAP_INT_SEARCHFS`，不支持的卷跳过。
- **错误容错**：某卷 `searchfs` 失败（常见于无 Full Disk Access 的 EPERM）时跳过该卷、
  继续其余卷；仅当所有卷均无结果且有错误时才返回错误。
- **Fyne GUI**：搜索框（回车触发）、选项行（仅文件/仅目录/大小写/上限）、可滚动结果列表
  （目录 📁 / 文件 📄 前缀）、底部状态行；搜索在后台 goroutine 执行，用 generation 计数
  丢弃过期结果，UI 更新经 `fyne.Do` 回到事件循环。
- **跨平台可编译**：非 darwin 平台有 `searchfs_other.go` 桩，保证 `go vet ./...` 与 CLI
  在任意平台可编译（`Search` 返回 “仅 macOS 可用” 错误）。

## TODO

- 结果按类型/路径排序，以及点击结果在 Finder 中显示（`open -R`）。
- `SRCHFS_SKIPPACKAGES` / `SRCHFS_SKIPINVISIBLE` / `SRCHFS_NEGATEPARAMS` 暴露到 GUI。
- `^`/`$` 前后缀锚定匹配（参考 `main.m` 的 startMatch/endMatch）。
- 输入防抖后的「边打边搜」增量搜索（当前为回车/按钮触发）。
- 用户自选卷（`-v`）、`--list` 列出可搜索卷。
- 单元测试（当前依赖 CI 冒烟）。
- `.app` 代码签名 / 公证（当前为未签名开发包）。

## CI Artifact 名称

**`road-a-go-app`** —— 内含 `MacFindAGo.app`（`fyne package` 产物）与原始二进制
`macfind-a-go`（保底可运行）。

## 目录结构

```
Road_A/Src_GO/
├── go.mod / go.sum
├── main.go                         入口：GUI 默认，--cli 走一次性 CLI 搜索
├── gui.go                          Fyne GUI（搜索框 + 选项 + 结果列表 + 状态行）
├── Icon.png                        .app 图标（fyne package 用）
├── internal/engine/
│   ├── options.go                  Options / Result / MatchKind 等可移植类型
│   ├── match.go                    大小写敏感后过滤、结果上限常量
│   ├── searchfs_darwin.go          cgo 封装 searchfs(2)（核心，仅 darwin）
│   └── searchfs_other.go           非 darwin 桩，保证可编译
└── README.md
```

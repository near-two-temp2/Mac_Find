# Road A · C++ — searchfs() 实时文件名搜索 GUI

macOS 秒速文件搜索的 **路线 A / C++** 实现。后台引擎直接调用内核 `searchfs(2)`
系统调用，在 APFS/HFS+ 的 catalog（B-Tree）上**实时**按文件名搜索，**不建索引**、
每次查询都是全卷实时扫描。GUI 用 **Qt6 Widgets**。

参考实现：`../../../Open_Ref/searchfs/main.m`（BSD-3，借鉴了打包属性缓冲区、
`SRCHFS_*` 标志与 `SRCHFS_START`/`EAGAIN` 分页循环）。

## 组成

| 文件 | 说明 |
|------|------|
| `SearchEngine.h` / `.cpp` | 纯 C++/POSIX 的 `searchfs()` 封装，GUI 与 CLI 共用（无 Qt 依赖） |
| `main_gui.cpp` | Qt6 GUI：搜索框 + 结果列表 + 选项，搜索在 `QThread` 后台线程流式执行 |
| `main_cli.cpp` | 无头 CLI 入口，供 CI 冒烟测试 |
| `CMakeLists.txt` | CMake 构建：静态引擎库 + CLI 可执行 + `.app` bundle |

## 已实现

- `searchfs()` 内核目录搜索（比 `find` 快约 100×）。
- **仅文件 / 仅目录**（`SRCHFS_MATCHFILES` / `SRCHFS_MATCHDIRS`，互斥）。
- **子串匹配**（默认，`SRCHFS_MATCHPARTIALNAMES`）与 **精确匹配**（`-e`）。
- **大小写敏感**：内核按大小写不敏感匹配，敏感模式在 basename 上做后置过滤。
- **结果上限**（跨卷累计后停止）。
- `fsgetpath()` 由 `fsid + objid` 还原绝对路径；对象在扫描间隙被删除时静默跳过。
- **EBUSY 重试**（catalog 变更时最多重试 5 次并重启扫描）。
- **Catalina+ 双卷**：未显式指定卷时，自动搜索 `/` 与 `/System/Volumes/Data`。
- 卷能力探测（`VOL_CAP_INT_SEARCHFS`）；CLI `-l` 列出支持 catalog 搜索的卷。
- GUI 后台线程 + 输入防抖（250ms）+ 流式追加结果 + 查询可取消。

## 构建

权威构建在 GitHub Actions（`macos-latest`）。本地（macOS + Qt6）亦可：

```bash
# 需要 Qt6：brew install qt@6 cmake
cmake -B build -S . -DCMAKE_PREFIX_PATH="$(brew --prefix qt@6)"
cmake --build build -j
open build/MacFindRoadACpp.app        # GUI
./build/macfind-a-cli -m 10 report    # CLI 冒烟
```

无 Qt 时 CMake 会自动跳过 GUI，仅构建引擎库与 CLI（便于纯引擎验证）。

### CLI 用法

```
macfind-a-cli [-dfesl] [-m limit] [-v volume] search_term
  -d 仅目录   -f 仅文件   -e 精确匹配   -s 大小写敏感
  -m 上限N    -v 指定卷挂载点   -l 列出可搜索卷
```

## CI

- Workflow：`../../.github/workflows/build-a-cpp.yml`
- 触发：`push`（仅 `Road_A/Src_CPP/**` 变更）与 `workflow_dispatch`。
- Runner：`macos-latest`，用 Homebrew 装 `qt@6`，CMake 编译出 `.app`。
- **Artifact 名：`road-a-cpp-app`**（含 `MacFindRoadACpp.app` 与 `macfind-a-cli`）。

## TODO

- 权限：全盘搜索需要「完全磁盘访问」；GUI 尚未引导用户授权（目前静默返回可访问范围内结果）。
- 无签名/公证：`.app` 未签名，CI artifact 仅供内部验证，直接运行可能被 Gatekeeper 拦截。
- 选项未接线：`skipPackages` / `skipInvisibles` 引擎已支持，但 GUI 尚未暴露开关。
- `^`/`$` 前后缀锚定（参考实现有）尚未移植到引擎/GUI。
- 结果列表未做双击「在 Finder 中显示」/打开等交互，也未按卷去重。
- APFS 上 `searchfs()` 比 HFS+ 慢 5–6×（Apple 已知退化），暂无缓解措施。

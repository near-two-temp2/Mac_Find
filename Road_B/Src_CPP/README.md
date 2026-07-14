# Road_B · C++ — 自建二进制索引 + fzf 模糊搜索（Qt6 GUI）

macOS 极速文件搜索的 **路线 B / C++** 实现：自建 mmap 友好的二进制索引 +
两阶段 fzf 模糊搜索，Qt6 桌面 GUI（搜索框 + 结果列表 + 「建索引」按钮）。

引擎设计参考 Cling（见 `../../../open-source-analysis.md` §3）：并行数组索引、
64-bit 字母 bitmask 预过滤、扩展名过滤、fzf 评分排序。

## 架构

```
src/
├── index_format.hpp   二进制索引头 + bitmask 编码（a-z / 0-9 / . - _）
├── fzf.hpp            fzf 评分：锚点枚举 + 贪婪匹配 + 边界/连续奖励 + 间隙惩罚
├── scanner.hpp/.cpp   fts(3) + FTS_NOSTAT 文件系统遍历（跳过 .git/node_modules 等）
├── index_engine.hpp/.cpp  核心引擎：build / save / mmap-load / 两阶段并行 search
├── paths.hpp          默认索引路径 ~/Library/Caches/com.mff.roadb-cpp/index.idx
├── main_cli.cpp       CLI 入口：index / search / selftest（CI 冒烟测试用）
└── main_gui.cpp       Qt6 GUI：QLineEdit + QListWidget + 后台建索引线程
```

### 二进制索引格式（并行数组，mmap 零拷贝）

| 数组 | 类型 | 用途 |
|------|------|------|
| `masks[]` | UInt64 | 整路径字母 bitmask（Phase-1 预过滤） |
| `bnMasks[]` | UInt64 | basename 字母 bitmask |
| `bnBoundaries[]` | UInt64 | basename 词边界位图（fzf 边界奖励） |
| `byteOffsets[]` | UInt32 | 路径在 `allBytes` 中的偏移 |
| `byteLengths[]` | UInt16 | 路径字节长度 |
| `bnStarts[]` | UInt16 | basename 在路径中的起始位置 |
| `extIds[]` | UInt16 | 扩展名 ID |
| `segCounts[]` | UInt8 | 路径段数（深度过滤） |
| `isDirs[]` | UInt8 | 是否目录 |
| `allBytes[]` | bytes | 打包的**小写** UTF-8 路径 blob |

### 两阶段搜索

- **Phase 1（O(n) 并行预过滤）**：按 CPU 核数分块，每块一个线程。
  单条 `masks[i] & qMask != qMask` 的 UInt64 比较即可排除绝大多数候选；
  再叠加 文件/目录 类型过滤、扩展名过滤（从字节实时派生，mmap 索引也可用）。
- **Phase 2（fzf 评分）**：对存活候选做模糊评分，优先匹配 basename（带边界奖励），
  失败再退回整路径匹配；按 分数 → 路径长度 → 字典序 排序，截断到 `maxResults`。

## 构建

**权威构建在 GitHub Actions（`macos-latest`）**，本地 macOS 12 不编译。

```bash
# 需要 Qt6（CI 用 Homebrew: brew install qt）
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_PREFIX_PATH="$(brew --prefix qt)"
cmake --build build -j
```

产物：
- `build/MacFindRoadB.app` — Qt6 GUI 应用
- `build/mff-b` — CLI 工具（`index` / `search` / `selftest`）

> Qt6 缺失时 CMake 会自动**只编译 CLI**（跳过 GUI），方便无 Qt 环境验证引擎。

## CLI 用法（脚本 / CI 冒烟测试）

```bash
mff-b index  [--root DIR]... [--out FILE]              # 建索引（默认索引 $HOME）
mff-b search [--index FILE] [--limit N] \
             [--files|--dirs] [--ext EXT] QUERY         # 查索引
mff-b selftest                                          # 自检（CI 用）
```

`selftest` 在临时目录建一棵小树，断言 fuzzy 匹配、扩展名过滤、save+mmap 往返全部通过。

## 已实现

- [x] fts + FTS_NOSTAT 全文件系统遍历，跳过噪声目录
- [x] Cling 式并行数组二进制索引（bitmask / 扩展名 / basename 边界）
- [x] `save()` 序列化 + `loadMmap()` 零拷贝加载（含 magic / 大小校验）
- [x] 两阶段搜索：多线程 bitmask 预过滤 + fzf 评分排序
- [x] 文件/目录 过滤、扩展名过滤、结果上限
- [x] Qt6 GUI：搜索框即时搜索 + 「Build Index」后台线程建索引 + 状态栏
- [x] 启动时自动加载上次保存的 `.idx`
- [x] CLI `index`/`search`/`selftest` 入口（本地已跑通，含 fuzzy + ext + mmap 往返）

## TODO

- [ ] **原始大小写显示**：当前索引只存小写字节，结果显示为小写路径。
      需并存原始路径 blob（或存大小写偏移）以还原显示。
- [ ] FSEvents 增量更新（当前需手动「Build Index」重建）
- [ ] SIMD 加速 fzf 锚点扫描（当前为标量 byte 扫描）
- [ ] 多卷 / `/System/Volumes/Data` 双卷扫描（当前默认只扫 `$HOME`）
- [ ] 词边界位图未在整路径回退分支使用（仅 basename 分支用了）
- [ ] GUI 搜索防抖 + 结果右键菜单（Finder 中显示 / Quick Look）
- [ ] 大索引下 search 结果流式回填 UI（当前一次性 addItem）

## CI

- Workflow：`.github/workflows/build-b-cpp.yml`
- 触发：`push`（仅 `Road_B/Src_CPP/**` 变更）、`workflow_dispatch`
- Runner：`macos-latest`；Homebrew 装 Qt6 → CMake 编译 CLI + `.app` → 跑 `selftest` 冒烟
- **Artifact 名：`road-b-cpp-app`**（含 `MacFindRoadB.app` 与 `mff-b` CLI）

# Road_B · Rust — 自建二进制索引 + fzf 模糊搜索（egui GUI）

macOS 极速文件搜索的 **路线 B / Rust 实现**：自建 mmap 二进制索引 + 并行 bitmask 预过滤 + fzf 模糊评分，
配一个 egui/eframe 的桌面 GUI（搜索框 + 结果列表）。架构参考 Cling（见
`../../../open-source-analysis.md` §3）。

## 架构一览

```
遍历文件系统 (walkdir)
        │
        ▼
建索引 (IndexWriter) ──► 二进制索引文件 (.idx，mmap 友好的并行数组)
        │                    ├─ masks[]    : u64  路径字母 bitmask
        │                    ├─ bnMasks[]  : u64  basename 字母 bitmask
        │                    ├─ bounds[]   : u64  词边界位图
        │                    ├─ meta[]     : offset/len/bn_start/ext_id/is_dir
        │                    └─ allBytes   : 打包的小写 UTF-8 路径字节
        ▼
mmap 加载 (IndexReader, memmap2) ──► 零拷贝切片视图
        │
        ▼
两阶段搜索 (search)
   Phase 1: rayon 并行 → bitmask 预检 + 扩展名 ID + 目录/文件过滤 → 存活候选下标
   Phase 2: 对存活候选并行 fzf 评分（多锚点 + 连续/边界奖励 + 间隙惩罚）→ 降序 top-N
```

位掩码编码（对齐 Cling）：`bit 0-25 = a-z`、`bit 26-35 = 0-9`、`bit 36 = '.'`、`bit 37 = '-'`、`bit 38 = '_'`。
查询时 `entry_mask & query_mask == query_mask` 一条指令即可 O(1) 排除不可能候选。

## 目录结构

```
Src_RUST/
├── Cargo.toml
├── src/
│   ├── lib.rs        库入口 + 默认索引路径 (~/Library/Caches/com.haifind.b-rust/index.idx)
│   ├── bitmask.rs    64-bit 字母 bitmask 编码 + 词边界
│   ├── index.rs      二进制索引格式：IndexWriter（建）/ IndexReader（mmap 读）
│   ├── search.rs     两阶段搜索：rayon 预过滤 + fzf 评分
│   └── bin/
│       ├── gui.rs      egui/eframe GUI（搜索框 + 结果列表 + 建索引按钮）
│       ├── index.rs    CLI：haifind-index（建索引，供 CI 冒烟）
│       └── search.rs   CLI：haifind-search（查索引，供 CI 冒烟）
└── README.md
```

## 构建方式

权威构建在 GitHub Actions 的 `macos-latest` runner（见 `.github/workflows/build-b-rust.yml`）。
本地（需 Rust stable）：

```bash
cargo build --release
cargo test --release
```

产物（`target/release/`）：

| 二进制 | 说明 |
|--------|------|
| `haifind-gui`    | egui GUI 桌面 app（CI 会打包成 `.app`） |
| `haifind-index`  | CLI：建索引 |
| `haifind-search` | CLI：查索引 |

## 使用

### GUI

```bash
./target/release/haifind-gui
```

- 顶部搜索框：输入即时搜索（每次改动重跑两阶段搜索），底部状态栏显示结果数与查询耗时（ms）。
- 「建立/重建索引」按钮：后台线程遍历 `$HOME` 建索引，完成后自动加载并刷新结果。
- 「全部 / 仅文件 / 仅目录」下拉过滤。
- 结果行双击 → `open -R` 在 Finder 中定位。

### CLI（也用于 CI 冒烟）

```bash
# 建索引（缺省索引 $HOME → ~/Library/Caches/com.haifind.b-rust/index.idx）
./target/release/haifind-index ~/some/dir --out /tmp/my.idx --max 20000

# 查索引（模糊，AND 语义的空格多 token）
./target/release/haifind-search main.rs --index /tmp/my.idx --limit 20 --files
```

## 已实现

- [x] mmap 友好的二进制索引格式（并行数组 + header 偏移表），`memmap2` 零拷贝加载。
- [x] 64-bit 字母 bitmask（路径 + basename）+ 词边界位图 + 扩展名 ID。
- [x] `walkdir` 遍历文件系统建索引（不跟随符号链接，跳过无权限项，支持条目上限）。
- [x] Phase 1：`rayon` 并行 bitmask + 扩展名 + 目录/文件过滤。
- [x] Phase 2：fzf 评分（多锚点、连续/边界奖励、间隙惩罚），basename 优先、同分短路径优先。
- [x] egui/eframe GUI：即时搜索、后台建索引（含进度）、类型过滤、Finder 定位、查询耗时展示。
- [x] `haifind-index` / `haifind-search` CLI 入口（CI 冒烟）。
- [x] 单元测试：bitmask、索引读写往返、扩展名 ID、搜索排序 / 过滤 / bitmask 排除。

## TODO

- [ ] **FSEvents 增量更新**：当前是全量重建；进阶应监听 `/Users` 等路径做实时增量（对齐 Cling §3.5）。
- [ ] **SIMD 加速 fzf 锚点枚举**：目前锚点用标量线性扫描；可用 `std::simd` / SSE 每次比较 16 字节。
- [ ] **多卷 / 排除规则**：仅索引 `$HOME`；应枚举 `/Volumes/*`、支持 .gitignore 与默认排除组。
- [ ] **searchfs() 初始扫描**：用 `searchfs()` 代替 `walkdir` 做更快更完整的初始遍历（路线 C 的方向）。
- [ ] **索引分块 / 压缩**：大规模（百万级）时的内存与磁盘占用优化。
- [ ] **高亮匹配区间**：`Match` 已带 `match_start/end`，GUI 尚未按区间着色。
- [ ] **原始大小写路径**：索引存小写用于匹配，展示/Finder 定位对大小写敏感卷可能不精确；应另存原始路径。
- [ ] **代码签名 / 公证**：CI 打的 `.app` 未签名，首次打开需右键「打开」绕过 Gatekeeper。

## CI Artifact

- workflow：`.github/workflows/build-b-rust.yml`（`on: [push, workflow_dispatch]`，`paths` 只匹配 `Road_B/Src_RUST/**`；`runs-on: macos-latest`）。
- artifact 名：**`road-b-rust-app`**（含 `MacHaiFind-B-Rust.app`，内置 GUI + 两个 CLI 二进制）。
- CI 会先 `cargo build --release` + `cargo test`，再跑 index/search CLI 冒烟测试，最后打包上传。

## 许可

MIT（本实现）。参考架构来源 Cling 为 GPLv3——此处仅借鉴设计思路，未复制其代码。

# Road_B · Tauri — 自建二进制索引 + fzf 模糊搜索 GUI

macOS 极速文件搜索（对标 Windows Everything）的 **Road_B / Tauri** 实现：
**Rust 后端** 负责自建 mmap 二进制索引与两阶段 fzf 搜索，**TypeScript 前端** 渲染搜索框与结果列表。
后端通过 `#[tauri::command]` 把「建索引 / 搜索 / 状态」暴露给前端，前端用 `@tauri-apps/api` 的 `invoke` 调用。

> 架构参考 Cling（见 `../../../open-source-analysis.md` §3）。与同路线的 `Road_B/Src_RUST`（egui）
> **共用同一套二进制索引契约**（相同的磁盘布局、bitmask 编码、fzf 评分），便于跨语言/跨框架交叉验证。

---

## 技术架构

### Rust 后端（`src-tauri/`）

搜索引擎位于 `src-tauri/src/engine/`，三个子模块：

| 模块 | 职责 |
|------|------|
| `bitmask.rs` | 64-bit 字母 bitmask 编码（Phase 1 O(1) 预过滤基石）+ 词边界位图 |
| `index.rs` | 建索引（遍历文件系统 → 并行数组 → mmap 友好的二进制落盘）与 `memmap2` 零拷贝读取 |
| `search.rs` | 两阶段搜索：Phase 1 `rayon` 并行 bitmask/扩展名预过滤；Phase 2 对存活候选做 fzf 评分排序 |

**二进制索引格式**（小端，8 字节对齐，`~/Library/Caches/com.haifind.b-tauri/index.idx`）：

```
Header(64B): magic "HAIFIB1" · version · entry_count · bytes_len · 各区段偏移
并行数组:  masks[]  (路径字母 bitmask, u64)
           bnMasks[](basename 字母 bitmask, u64)
           bounds[] (词边界位图, u64)
           meta[]   (byte_offset/len, bn_start, ext_id, is_dir; 16B/条)
allBytes:  打包的小写 UTF-8 路径字节
```

**bitmask 编码**：bits 0-25 = a-z，26-35 = 0-9，36 = `.`，37 = `-`，38 = `_`。
查询时先算 `query_mask`，`entry_mask & query_mask != query_mask` 一条指令即可排除不可能候选（无假阴性）。

**fzf 评分**（对齐 Cling）：字符匹配 +16 · 连续 +4 · 首字符 ×2 · 词边界 +9 · 间隙开始 −3 · 间隙延续 −1；
多锚点枚举（最多 32 个），basename 命中额外 +12，同分时较短路径优先。

### 暴露给前端的 Tauri 命令（`src-tauri/src/lib.rs`）

| 命令 | 说明 |
|------|------|
| `build_index(roots, maxEntries, outPath)` | 遍历根路径（空则默认 `$HOME`）建索引，完成后即 mmap 加载 |
| `load_index(indexPath)` | 加载已有 `.idx`（启动时探测），不重建 |
| `index_status()` | 返回当前是否已加载索引、条目数、路径 |
| `search(query, kind, limit)` | 在已加载索引上两阶段搜索，返回结果 + 命中数 + 后端耗时 |

已 mmap 的索引存于 `AppState`（`Mutex<Option<IndexReader>>`），搜索无需每次重新映射。

### TypeScript 前端（`src/`）

- `index.html` — 搜索框 + 类型过滤（全部/文件/目录）+ 结果列表骨架。
- `src/main.ts` — `invoke` 调用后端命令；输入防抖（90ms）即时搜索；basename 命中高亮；
  单击「在 Finder 显示」、双击「打开」（经 `@tauri-apps/plugin-opener`）；底部显示命中数与后端耗时。
- `src/styles.css` — 暗色主题 UI。

---

## 构建方式

**权威构建在 GitHub Actions（`macos-latest`）**，见 `../../.github/workflows/build-b-tauri.yml`。
开发机为 macOS 12，不在本地编译；本仓库只写源码 + 配置 + 该实现自己的 workflow。

CI 步骤：装 Rust + Node 22 → `npm ci` → `cargo test --lib`（引擎单测）→
`haifind-tauri-search --self-test`（引擎冒烟）→ `npm run tauri build`（打 `.app` + `.dmg`）→ 上传 artifact。

### 本地开发（可选，需较新 macOS）

```bash
cd Road_B/Src_TAURI
npm install
npm run tauri dev          # 开发模式（热重载前端 + Rust 后端）
npm run tauri build        # 产出 .app + .dmg 到 src-tauri/target/release/bundle/

# 仅前端类型检查 + 打包
npm run build

# 引擎 CLI 冒烟测试（复用与 GUI 相同的引擎）
cd src-tauri
cargo build --release --bin haifind-tauri-search
./target/release/haifind-tauri-search --self-test          # 合成数据自检，必过
./target/release/haifind-tauri-search rs --root "$HOME" --max 20000 --limit 10
cargo test --lib          # 引擎单元测试
```

---

## CI Artifact

- **Artifact 名**：`road-b-tauri-app`
- **内容**：
  - `src-tauri/target/release/bundle/macos/*.app`（可运行 app bundle）
  - `src-tauri/target/release/bundle/dmg/*.dmg`（磁盘映像）

---

## 已实现

- [x] Rust 后端完整搜索引擎：mmap 二进制索引（并行数组 + bitmask + 词边界 + 扩展名 ID）。
- [x] 两阶段搜索：`rayon` 并行 bitmask/扩展名预过滤 + fzf 评分排序。
- [x] `#[tauri::command]` 暴露 `build_index` / `load_index` / `index_status` / `search`。
- [x] 已加载索引缓存在 `AppState`，避免每次搜索重新 mmap。
- [x] TypeScript 前端：搜索框 + 类型过滤 + 结果列表 + basename 高亮 + 防抖即时搜索。
- [x] 单击「Finder 中显示」、双击「打开」（`plugin-opener`）。
- [x] `haifind-tauri-search` CLI（含 `--self-test`）供 CI 冒烟测试，复用同一引擎。
- [x] 引擎单元测试（索引 roundtrip、ext_id、fzf 排序、bitmask 排除、kind 过滤）。
- [x] CI workflow：`macos-latest` 上编译打包并上传 `.app` / `.dmg`。

## TODO

- [ ] **FSEvents 增量更新**：目前索引是一次性快照，文件变动需手动「重建索引」。
      计划接入 `fsevent`/`notify` crate 实时追加/删除条目（对齐 Cling §3.5）。
- [ ] **多作用域索引**：当前把所有根路径合并进一个 `.idx`；可拆成 home/applications/system
      等独立引擎并行搜索（对齐 Cling §3.6）。
- [ ] **SIMD 锚点搜索**：Phase 2 的首字符锚点枚举目前是标量循环，可用 `std::simd` 16 字节并行。
- [ ] **进度反馈**：建索引期间通过 Tauri event 向前端推送进度条（当前只有开始/结束状态）。
- [ ] **保留原始大小写**：索引只存小写路径用于匹配；展示时应另存原始大小写字节。
- [ ] **searchfs() 兜底**：本路线 B 为纯索引；索引缺失时的实时兜底属 Road_C 范畴。
- [ ] **自定义排除规则 / .gitignore**：目前遍历全部条目（受 `--max` 限制），未做排除。
- [ ] **图标**：当前 `src-tauri/icons/icon.png` 为占位图，需替换为正式品牌图标（含 `.icns` 多分辨率）。

---

## 目录结构

```
Road_B/Src_TAURI/
├── package.json            前端依赖与脚本（dev / build / tauri）
├── vite.config.ts          Vite（端口 1420，产物 → dist/）
├── tsconfig.json
├── index.html              前端入口（搜索框 + 结果列表）
├── src/
│   ├── main.ts             invoke 调用后端命令 + 渲染逻辑
│   └── styles.css          暗色主题
└── src-tauri/
    ├── Cargo.toml          Rust crate（lib + GUI bin + CLI bin）
    ├── build.rs            tauri-build
    ├── tauri.conf.json     Tauri v2 配置（窗口 / bundle 目标 .app+.dmg）
    ├── capabilities/
    │   └── default.json    前端调用权限
    ├── icons/icon.png      占位图标
    └── src/
        ├── main.rs         GUI 入口 → haifind_tauri_lib::run()
        ├── lib.rs          Tauri 命令 + AppState + run()
        ├── bin/search.rs   haifind-tauri-search CLI（含 --self-test）
        └── engine/
            ├── mod.rs      引擎入口 + 默认索引路径
            ├── bitmask.rs  bitmask 编码 + 词边界
            ├── index.rs    建索引 + mmap 读取
            └── search.rs   两阶段 fzf 搜索
```

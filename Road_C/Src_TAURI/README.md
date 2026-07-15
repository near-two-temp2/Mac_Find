# Road_C · Tauri — macOS 混合文件搜索（完整版）

对标 Windows Everything 的 macOS 秒速文件搜索，**路线 C（混合引擎完整版）** 的 Tauri v2 实现：
**Rust 后端引擎 + TypeScript 前端 GUI**。

- **主引擎**：自建 mmap 友好的二进制索引（并行数组 + 64-bit 字母 bitmask 预过滤 + fzf 模糊评分），
  用 [rayon](https://docs.rs/rayon) 做多核并行的 Phase-1 扫描。
- **兜底引擎**：索引缺失 / 损坏 / 尚未构建时，透明降级到 macOS 内核 `searchfs(2)` 系统调用
  （Rust FFI 直调，移植自 `Open_Ref/searchfs/main.m` 的调用序列），保证 100% 准确。
- 引擎通过 `#[tauri::command]` 暴露给 TS 前端；前端用原生 TypeScript + Vite 渲染
  「搜索框 + 结果列表 + 在 Finder 中显示」。

## 架构

```
┌─────────────────────────────────────────────┐
│  TypeScript 前端 (Vite)                       │
│  搜索框 + 结果列表 + 选项 + 「在 Finder 中显示」 │
│    src/main.ts · api.ts · types.ts · styles   │
└───────────────┬───────────────────────────────┘
                │  invoke()  (Tauri IPC, camelCase JSON)
┌───────────────▼───────────────────────────────┐
│  Rust 后端 (src-tauri/)                         │
│  commands.rs  → #[tauri::command] 薄封装         │
│  engine/                                        │
│    mod.rs      混合编排：索引优先，searchfs 兜底 │
│    index.rs    自建二进制索引 (bitmask+rayon)     │
│    fzf.rs      Cling 式 fzf 评分                 │
│    searchfs.rs searchfs(2) FFI 兜底             │
│    types.rs    IPC 类型 (serde)                  │
└─────────────────────────────────────────────────┘
```

搜索决策（`engine/mod.rs::search`）：内存中有非空索引 → 走索引（Phase-1 bitmask 并行预过滤 +
Phase-2 fzf 评分排序）；否则调 `searchfs(2)` 实时扫 `/` 与 `/System/Volumes/Data`。

## 目录结构

```
Road_C/Src_TAURI/
├── package.json            前端依赖与脚本
├── vite.config.ts          Vite 配置（固定 5173 端口，输出到 dist/）
├── tsconfig.json           TypeScript 严格模式
├── index.html              GUI 外壳
├── src/                    前端 TypeScript
│   ├── main.ts             控制器：防抖搜索、渲染、事件
│   ├── api.ts              invoke() 封装
│   ├── types.ts            与 Rust serde 类型对应
│   └── styles.css          深色主题样式
└── src-tauri/              Rust 后端 + Tauri 配置
    ├── Cargo.toml          crate 清单（lib + 2 个 bin）
    ├── tauri.conf.json     Tauri v2 应用/打包配置
    ├── build.rs            tauri-build
    ├── capabilities/       Tauri v2 权限
    ├── icons/              应用图标
    └── src/
        ├── lib.rs          Tauri run() 入口 + 模块导出
        ├── main.rs         GUI 二进制入口
        ├── cli.rs          headless CLI（CI 冒烟测试用）
        ├── commands.rs     #[tauri::command] 处理器
        └── engine/         混合搜索引擎（见上图）
```

## 构建方式

**权威构建在 GitHub Actions 的 `macos-latest` runner 上**（开发机是 macOS 12，不本地编译）。

CI 步骤（见 `.github/workflows/build-c-tauri.yml`）：

```bash
# 前端依赖
npm ci

# Rust 单元测试（index + fzf）
cd src-tauri && cargo test --release --lib

# CLI 冒烟测试（不需要显示器 / Full Disk Access）
cargo build --release --bin mac-find-c-cli
./target/release/mac-find-c-cli --check
./target/release/mac-find-c-cli --status

# 打包 .app + .dmg
cd .. && npm run tauri build
```

产物（`src-tauri/target/release/bundle/`）：`macos/*.app` 与 `dmg/*.dmg`。

### 本地开发（可选）

需要 Rust ≥ 1.77 与 Node 22：

```bash
npm install
npm run tauri dev      # 热重载开发
npm run tauri build    # 本地打包
```

### CLI（脚本 / 冒烟测试）

```bash
cd src-tauri
cargo run --bin mac-find-c-cli -- --check            # 内建自检：建索引→加载→搜索
cargo run --bin mac-find-c-cli -- --status           # 打印引擎状态
cargo run --bin mac-find-c-cli -- --build ~/Documents  # 建索引
cargo run --bin mac-find-c-cli -- readme             # 走索引搜索
cargo run --bin mac-find-c-cli -- --live report      # 强制 searchfs 实时搜索
```

## GUI 功能

- 搜索框，输入即搜（120ms 防抖）
- 结果列表：图标 / 文件名 / 所在目录，按 fzf 分数排序
- 选项：仅文件 / 仅目录（互斥）、Live（强制 searchfs）
- **双击** 打开、**右键 / 空格** 在 Finder 中显示（`open -R`）、**Enter** 打开
- 「Build index」按钮：在默认根目录（`$HOME` + `/Applications`）构建索引
- 状态栏显示当前引擎（索引 / searchfs 兜底）与索引条目数
- 底栏显示：命中数 / 扫描候选数 / 耗时 / 使用的引擎

## 已实现

- ✅ 混合引擎编排：索引优先，`searchfs(2)` 兜底（`engine/mod.rs`）
- ✅ 自建二进制索引：并行数组布局、64-bit bitmask 预过滤、rayon 并行 Phase-1、写入用 temp+rename 防损坏（`engine/index.rs`）
- ✅ fzf 评分：字符匹配 / 连续奖励 / 词边界（分隔符·空格·camelCase）/ 间隙惩罚（`engine/fzf.rs`）
- ✅ `searchfs(2)` FFI 兜底：双卷（`/` + `/System/Volumes/Data`）、EBUSY 重试、EAGAIN 循环、`fsgetpath` 还原路径（`engine/searchfs.rs`）
- ✅ 6 个 `#[tauri::command]`：`search` / `search_live` / `engine_status` / `rebuild_index` / `reveal_in_finder` / `open_path`
- ✅ TS 前端：防抖搜索、结果渲染、打开 / Finder 显示、构建索引、引擎状态展示
- ✅ CLI 冒烟入口 + Rust 单元测试（index roundtrip、corrupt 检测、fzf 排序）
- ✅ CI workflow：macos-latest 上 test + CLI check + `tauri build` + 上传 .app/.dmg

## TODO

- ⬜ **真正的 mmap**：当前 `Index::load` 一次性读入内存（布局仍是 mmap 友好的）；改用 `memmap2` 零拷贝映射可降低启动内存。
- ⬜ **保留原始大小写**：索引 blob 存的是小写路径，结果显示为小写。生产版应旁存原始大小写字节（现为已知 TODO）。
- ⬜ **FSEvents 增量更新**：目前索引靠手动「Build index」重建；应加 FSEvents 监听做实时增量（参考 Cling `FuzzyClient.swift`）。
- ⬜ **异步重建**：`rebuild_index` 命令同步阻塞；应放到后台任务并向前端推进度事件。
- ⬜ **扩展名 ID / 词边界位图**：Phase-1 尚未做扩展名过滤与词边界预存，可进一步加速与提分。
- ⬜ **searchfs 初始扫描**：初始索引用 `walkdir`；按 §5.4 建议可改用 `searchfs()` 初始全卷扫描，更快更全。
- ⬜ **多卷 / 作用域**：默认只索引 `$HOME` + `/Applications`；应支持外部卷与可配置作用域。
- ⬜ **代码签名 / 公证**：CI 产出的 .app/.dmg 未签名，本地运行需右键打开或 `xattr -dr com.apple.quarantine`。

## CI Artifact

- **workflow**：`.github/workflows/build-c-tauri.yml`
- **artifact 名**：`road-c-tauri-app`
- **内容**：`src-tauri/target/release/bundle/macos/*.app` 与 `bundle/dmg/*.dmg`

## 参考

- `../../../open-source-analysis.md` §3（Cling 索引）、§5.4（推荐混合架构）
- `../../../Open_Ref/searchfs/main.m`（searchfs 兜底引擎参考实现）

## 许可

参考实现衍生：searchfs 部分借鉴 BSD-3-Clause 的 `sveinbjornt/searchfs`；fzf 评分思路参考 GPLv3 的 Cling。
本目录代码为调研用途。

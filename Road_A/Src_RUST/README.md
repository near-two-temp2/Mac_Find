# Road_A · Rust — macOS 极速文件名搜索（searchfs，无索引）

对标 Windows Everything 的 macOS 秒速文件名搜索，**路线 A / Rust 实现**。

一个 **egui/eframe GUI 桌面 app**：搜索框 + 结果列表。后台引擎通过 `libc`
FFI 直接调用 macOS 内核的 **`searchfs(2)`** 系统调用，在 APFS/HFS+ 的 B-Tree
catalog 上实时搜索文件名——**不建索引，每次查询都是一次内核级实时扫描**，比
`find` 快约 100 倍。命中的 `(fsid, objid)` 用 `fsgetpath` 私有 SPI 还原为绝对路径。

参考实现：`../../../Open_Ref/searchfs/main.m`（BSD-3，653 行 C）。本实现是它在
Rust + egui 上的移植与 GUI 化。

## 目录结构

```
Road_A/Src_RUST/
├── Cargo.toml            依赖与两个二进制目标（GUI / CLI）+ lib
└── src/
    ├── searchfs_sys.rs   searchfs()/fsgetpath()/getattrlist() 原始 FFI 声明
    ├── engine.rs         安全封装：搜索循环 + EBUSY 重试 + 双卷 + 过滤
    ├── lib.rs            共享库，GUI 与 CLI 复用同一引擎
    ├── main.rs           egui/eframe GUI（搜索框 + 结果列表）
    └── cli.rs            CLI 入口（CI 冒烟测试 / 脚本用）
```

## 构建方式

> 权威构建在 **GitHub Actions `macos-latest`** 上完成（见
> `../../.github/workflows/build-a-rust.yml`）。开发机为 macOS 12，不在本地强制编译。

本地（macOS）可选试编译：

```bash
cd Road_A/Src_RUST
cargo build --release            # 产出 GUI + CLI 两个二进制
cargo run --bin mac-find-gui     # 启动 GUI
cargo run --bin mac-find-cli -- --self-test   # CLI 自检
cargo run --bin mac-find-cli -- -f -m 20 report   # 搜 "report"，仅文件，上限 20
```

产物：
- `target/release/mac-find-gui` — GUI 桌面 app 可执行文件
- `target/release/mac-find-cli` — CLI 冒烟/脚本工具

CI 另外把 GUI 打包成 `MacFind-RoadA-Rust.app` bundle 一并上传。

## CI Artifact

- Workflow：`.github/workflows/build-a-rust.yml`
- **Artifact 名：`road-a-rust-app`**
- 内容：`.app` bundle（`dist/MacFind-RoadA-Rust.app`）+ `mac-find-gui` +
  `mac-find-cli` 裸二进制。
- 触发：`push`（仅 `Road_A/Src_RUST/**` 变更）与 `workflow_dispatch`；
  `runs-on: macos-latest`。

## CLI 用法

```
mac-find-cli [-dfesm] [--self-test] <search_term>

    -d, --dirs-only        仅匹配目录
    -f, --files-only       仅匹配文件
    -e, --exact-match      精确文件名匹配（非子串）
    -s, --case-sensitive   区分大小写
    -m, --limit <N>        命中 N 个后停止（0 = 不限）
        --self-test        运行内部自检并退出（CI 用）
    -h, --help             帮助
```

## 已实现

- ✅ `searchfs(2)` 原始 FFI（结构体 / `SRCHFS_*`、`ATTR_*` 常量 / `extern "C"`
  原型全部手写，`libc` 未提供）。
- ✅ 完整搜索循环：`SRCHFS_START` → `EAGAIN` 续搜 → `EBUSY` 有限次重试
  （catalog 变化，与参考 C 的 `catalog_changed` 语义一致）。
- ✅ `fsgetpath` 把 `(fsid, objid)` 还原为绝对路径。
- ✅ Catalina+ 双卷搜索：默认同时搜 `/` 与 `/System/Volumes/Data`
  （用 `getattrlist` + `VOL_CAP_INT_SEARCHFS` 探测卷是否支持 searchfs）。
- ✅ 搜索选项：仅文件 / 仅目录、子串匹配、精确匹配、大小写、结果上限。
- ✅ egui GUI：搜索框 + 选项行（复选框 + limit 输入）+ 可滚动结果列表；
  后台线程搜索、generation 计数丢弃过期结果、双击「在 Finder 中显示」。
- ✅ CLI 入口 + `--self-test`，供 CI 冒烟。
- ✅ 非 macOS 目标下引擎降级为空实现，保证 crate 在任何 host 上都能 `cargo check`。

## TODO

- [ ] `-p/--skip-packages`、`-i/--skip-invisibles`、`-n/--negate-params`
  选项（引擎已预留 `SRCHFS_*` 常量，尚未接到 UI）。
- [ ] `^`/`$` 前后缀锚定匹配（当前仅 exact / substring）。
- [ ] 用户可选具体卷（`-v`），当前固定搜默认双卷。
- [ ] 大结果集的增量流式回填 UI（当前一次性返回后再渲染；已用
  `show_rows` 虚拟化列表，滚动性能 OK）。
- [ ] 代码签名 / 公证 + Full Disk Access 引导（无 FDA 时结果不完整，
  非崩溃）。
- [ ] 命中项的 `stat` 是逐条 `std::fs::metadata`，超大结果集可考虑批量或省略。

## 备注

- **权限**：完整扫描需要「完全磁盘访问权限」（Full Disk Access）。未授权时
  `searchfs` 仍可运行，只是结果可能不完整——不会崩溃，CI 自检以「引擎跑通、
  零命中也算通过」为准。
- **性能提示**：APFS 上 `searchfs()` 比 HFS+ 慢 5-6 倍（Apple 已知退化），
  但仍远快于 `find`。

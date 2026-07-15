# Road_C · Rust — macOS 极速文件搜索 GUI（混合引擎完整版）

对标 Windows Everything 的 macOS 秒速文件搜索，Rust + egui/eframe 桌面 app。
这是「18 实现矩阵」里 **路线 C（混合完整版）× Rust** 的实现。

## 混合引擎（本路线的核心）

```
             ┌───────────────┐
   query ──▶ │  HybridEngine │
             └──────┬────────┘
                    │ 索引可用（mmap 打开成功且非空）？
          ┌─────────┴──────────┐
         是                    否 / 损坏 / 空
          ▼                     ▼
   ┌─────────────┐      ┌──────────────────────┐
   │ 索引引擎(主) │      │ searchfs() 实时兜底(备) │
   │ Phase1 rayon │      │ libc FFI 调内核 catalog│
   │ 并行 bitmask │      │ 100% 准确、无需索引     │
   │ Phase2 fzf   │      └──────────────────────┘
   └─────────────┘
```

- **主路径**：自建 mmap 二进制索引 + rayon 并行 64-bit bitmask 预过滤 + fzf 模糊评分。
- **兜底路径**：索引缺失/损坏/为空时，自动降级到 `searchfs(2)` 系统调用（libc FFI）实时扫描
  `/` 与 `/System/Volumes/Data`（Catalina+ 双卷），保证「没索引也能搜」。
- GUI 顶部有后端状态徽标，实时显示当前走的是「索引主路径」还是「searchfs 实时兜底」。

架构对齐 `../../../open-source-analysis.md` §3（Cling 索引）与 §5.4（推荐混合架构）。
searchfs 调用序列参考 `../../../Open_Ref/searchfs/main.m`。

## 目录结构

```
Road_C/Src_RUST/
├── Cargo.toml           # lib + gui bin + cli bin
├── src/
│   ├── lib.rs           # 模块粘合 + 对外 API
│   ├── bitmask.rs       # 64-bit 字母 bitmask 编码（与 Road_B 一致）
│   ├── fuzzy.rs         # fzf 风格模糊评分
│   ├── index.rs         # mmap 二进制索引：IndexWriter / IndexReader
│   ├── searchfs.rs      # searchfs() FFI + 实时兜底引擎（cfg(macos)）
│   ├── engine.rs        # HybridEngine：主/兜底编排 + 两阶段搜索 + 建索引
│   ├── reveal.rs        # 在 Finder 中显示 / 打开
│   ├── main.rs          # egui/eframe GUI（后台线程搜索，UI 不卡）
│   └── cli.rs           # CLI：index / search / doctor（CI 冒烟用）
└── (CI: ../../.github/workflows/build-c-rust.yml)
```

## 构建方式

**权威构建在 GitHub Actions 的 `macos-latest` runner 上**（开发机为旧版 macOS，不本地编译）。
workflow：`.github/workflows/build-c-rust.yml`，`paths` 只匹配 `Road_C/Src_RUST/**`。
触发：`push`（改到本目录时）或 Actions 页手动 `workflow_dispatch`。

CI 步骤：装 rustup → `cargo build --release --bins` → `cargo test` →
CLI 冒烟（index → search → doctor）→ 打包 `.app` → 上传 artifact。

本地（macOS）想试：

```bash
cargo build --release --bins      # 产出 haifind-c-gui / haifind-c-cli
cargo test                        # 跑单测
cargo run --bin haifind-c-gui     # 启动 GUI

# CLI 冒烟
cargo run --bin haifind-c-cli -- index --root src --out /tmp/t.idx
cargo run --bin haifind-c-cli -- search --index /tmp/t.idx main
cargo run --bin haifind-c-cli -- doctor
```

## CI Artifact

- **artifact 名：`road-c-rust-app`**
- 内容：`dist/HaiFind-C.app`（GUI 应用包）+ `haifind-c-gui` / `haifind-c-cli` 可执行文件。

## 已实现

- ✅ 混合引擎：索引主路径 + searchfs() 实时兜底，自动降级。
- ✅ mmap 二进制索引：并行数组格式，原子写（临时文件 + rename），头部魔数/版本/长度校验（损坏即降级）。
- ✅ 两阶段搜索：rayon 并行 bitmask 预过滤 → fzf 评分排序（basename 命中加权）。
- ✅ searchfs() FFI 兜底：EAGAIN 续搜、EBUSY 有界重试、双卷、`fsgetpath` 还原路径。
- ✅ egui GUI：搜索框（输入即搜）+ 仅文件/仅目录/结果上限 + 结果列表 + 每行「在 Finder 中显示」/「打开」+ 后端状态徽标 + 建/重建索引按钮。
- ✅ 后台工作线程：搜索与建索引不阻塞 UI；旧查询结果被新查询丢弃。
- ✅ CLI：`index` / `search` / `doctor` 三子命令，供脚本与 CI 冒烟。
- ✅ 单元测试：bitmask、fuzzy、index 读写往返、engine 建索引+搜索。

## TODO

- ⏳ FSEvents 增量更新（当前索引为一次性全量重建；变更后需手动「重建索引」）。
- ⏳ 索引后台自动老化重建（Cling 式定期全量重建）。
- ⏳ 扩展名 ID 字段 + 快速过滤器（当前只按名字/路径子串 + 文件/目录）。
- ⏳ SIMD 加速 fzf 锚点扫描（当前为标量 `memchr`）。
- ⏳ 建索引进度回传 GUI（当前只显示「建索引中…」spinner）。
- ⏳ Everything 风格精确通配符 / 正则模式。
- ⏳ `.gitignore` / 默认排除规则（当前遍历全部条目）。
- ⏳ `.app` 代码签名 / 公证（CI 产出未签名，本地运行需右键打开或允许）。

## 许可证

MIT（本实现）。searchfs 调用序列参考 BSD-3 的 `Open_Ref/searchfs`。

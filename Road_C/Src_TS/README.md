# Road_C · TypeScript — 混合引擎 GUI（完整版）

macOS 极速文件搜索的 **Road_C（TypeScript / Electron）** 实现。对标 Windows Everything，
后台采用 **混合搜索引擎**：自建二进制索引为主，`searchfs()` 实时兜底。

这是 18 实现矩阵中的一个（路线 C × TypeScript）。GUI 外壳与 Road_A/B 的 TS 版一致，
差别只在搜索引擎。

## 形态

- **Electron + TypeScript** 桌面 app（`MacFindRoadCTs.app`）
- UI：搜索框（防抖）+ 结果列表 + 类型过滤（全部/文件/文件夹）+ **在 Finder 中显示** / 打开
- 顶部徽章实时显示当前引擎模式：`index`（走索引）/ `searchfs`（兜底）/ `none`
- 附带 CLI 入口（`macfind-c-cli`）供脚本与 CI 冒烟测试

## 架构：混合引擎

对应 `../../open-source-analysis.md` §5.4 推荐的混合方案：

```
搜索请求
  ├─ 索引存在？ ──► 主路径：自建二进制索引
  │                 · Phase 1  bitmask 预过滤（O(n) 并行）
  │                 · Phase 2  fzf 模糊评分（存活候选）
  │                 · worker_threads 分片并行，合并各分片 top-K
  └─ 索引缺失/损坏 ─► 兜底路径：native addon 调 searchfs()
                       · 内核级 catalog 实时搜索，100% 准确
                       · 双卷（/ 与 /System/Volumes/Data）
```

### 关键模块（`src/engine/`）

| 文件 | 职责 |
|------|------|
| `bitmask.ts` | 64-bit 字符类别位掩码（a-z / 0-9 / `.` `-` `_`），O(1) 预过滤 |
| `fuzzy.ts` | fzf 风格子序列评分（词边界奖励 / 连续奖励 / 间隙惩罚） |
| `binaryIndex.ts` | 并行数组二进制索引格式（typed array，mmap 友好），build / load |
| `scanner.ts` | 文件系统遍历，产出索引条目（默认排除 .git/node_modules/Caches 等） |
| `query.ts` | 两阶段查询：预过滤 + 评分 + top-K，可按 `[start,end)` 分片 |
| `searchWorker.ts` | `worker_threads` 入口，每个 worker 负责一个索引分片 |
| `indexStore.ts` | 索引持久化到 `~/Library/Caches/org.macfind.roadc.ts/index.idx` |
| `searchfsFallback.ts` | 加载 native addon；addon 不存在时优雅降级 |
| `hybridEngine.ts` | 编排：索引优先，缺失则 searchfs 兜底，报告使用了哪条路径 |

### 二进制索引格式（`binaryIndex.ts`）

参考 Cling 的 `.idx` 布局（§3.3）：Header(32B) + 并行数组（maskLo/maskHi/byteOffset/
byteLen/baseStart/isDir）+ 打包的小写路径字节 blob（外加同偏移的原始大小写 blob 供 UI 展示）。
掩码拆成两个 UInt32 word，查询时纯 32-bit 整数比较，避免 BigInt。

### native searchfs addon（`native/searchfs_addon.mm`）

N-API（node-addon-api）封装 macOS `searchfs()` 系统调用，调用序列、`fssearchblock`
结构、`fsgetpath()`、EBUSY 重试与双卷逻辑改编自参考实现
`../../Open_Ref/searchfs/main.m`（BSD-3-Clause）。由 `node-gyp`（`binding.gyp`）编译为
`build/Release/searchfs_addon.node`。

## 构建方式

> 权威构建在 GitHub Actions 的 `macos-latest` runner 上（见下）。本地可试跑（需 macOS + Xcode CLT + Node 22）。

```bash
npm install              # 安装依赖（含 electron / electron-builder / node-gyp）
npm run build            # ① 编译 native addon  ② tsc  ③ 拷贝 renderer 资源
npm start                # 本地运行 GUI（Electron）
npm run dist             # electron-builder 打包 .app / .dmg 到 release/
```

CLI（CI 冒烟 / 脚本）：

```bash
node dist/cli/cli.js --help
node dist/cli/cli.js status                    # 索引 + addon 状态
node dist/cli/cli.js index ~ --max 100000      # 建索引
node dist/cli/cli.js search foo --limit 20     # 混合搜索
```

## CI

Workflow：`.github/workflows/build-c-ts.yml`

- 触发：`push`（仅 `Road_C/Src_TS/**` 变更）+ `workflow_dispatch`
- `runs-on: macos-latest`
- 步骤：`setup-node@22` → `npm install` → 编译 addon → `tsc` + 拷贝资源 →
  CLI 冒烟（help / status / index / search）→ `electron-builder` 打包 → 上传 artifact
- **Artifact 名：`road-c-ts-app`**（包含 `release/**/*.app` 与 `*.dmg`）

## 已实现

- [x] 完整混合引擎：自建二进制索引（typed array + bitmask 预过滤 + fzf 评分）
- [x] `worker_threads` 分片并行搜索 + 各分片 top-K 合并
- [x] 索引缺失/损坏时自动降级到 native `searchfs()` 兜底
- [x] Electron GUI：搜索框（防抖）+ 结果列表 + 类型过滤 + 引擎模式徽章
- [x] 在 Finder 中显示 / 打开（`shell.showItemInFolder` / `shell.openPath`）
- [x] 建索引按钮（后台重扫 + 重建 worker）
- [x] CLI 入口（help / status / index / search）
- [x] `binding.gyp` + N-API searchfs addon，`electron-builder` 打包配置
- [x] 本地验证：TS 编译通过、addon 在 macOS 12 编译通过、索引路径与 searchfs 兜底路径均实测可用

## TODO

- [ ] FSEvents 增量更新（当前重扫为全量；参考 §3.5）— 需要 `chokidar` 或 native FSEvents 绑定
- [ ] 扩展名 ID 预过滤字段（当前索引仅做字符类掩码，未存 extID）
- [ ] SIMD / 更激进的锚点枚举（当前 fzf 为标量贪婪匹配）
- [ ] searchfs addon 返回文件/目录类型（当前兜底结果 `isDir` 统一为 false，UI 图标近似）
- [ ] 大索引改用 `SharedArrayBuffer` 零拷贝共享给 worker（当前每 worker 克隆一份 buffer）
- [ ] 索引分作用域（home / applications / external）与后台定期重建
- [ ] 代码签名 / 公证（CI 中 `identity: null`，产物未签名，本地打开需右键“打开”）
- [ ] electron-builder 目前仅打 `arm64`；如需 Intel 增加 `x64` target

## 许可证

- 本实现：BSD-3-Clause
- `native/searchfs_addon.mm` 的 searchfs 调用逻辑改编自 sveinbjornt/searchfs（BSD-3-Clause）

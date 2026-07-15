// Road_B (Tauri) 前端逻辑（TypeScript）
//
// 通过 @tauri-apps/api 的 invoke 调用 Rust 后端暴露的 #[tauri::command]：
//   - index_status()          启动时探测是否已有可用索引
//   - build_index(roots, ...) 建立/重建二进制索引
//   - search(query, kind, …)  两阶段 fzf 搜索
//
// UI：搜索框（输入即搜，带防抖）+ 类型过滤（全部/文件/目录）+ 结果列表
//     （高亮 basename、显示分数），底部状态栏显示命中数与后端耗时。

import { invoke } from "@tauri-apps/api/core";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

// ── 与 Rust 端 #[derive(Serialize)] 对应的类型 ────────────────────────
interface MatchItem {
  index: number;
  score: number;
  path: string; // 小写路径（后端存的是小写字节）
  is_dir: boolean;
  match_start: number;
  match_end: number;
}

interface SearchResponse {
  results: MatchItem[];
  total: number;
  elapsed_ms: number;
}

interface IndexResult {
  entry_count: number;
  bytes_len: number;
  file_size: number;
  index_path: string;
  elapsed_ms: number;
}

interface IndexStatus {
  loaded: boolean;
  entry_count: number;
  index_path: string;
}

type Kind = "all" | "files" | "dirs";

// ── DOM 句柄 ──────────────────────────────────────────────────────────
const $ = <T extends HTMLElement>(sel: string): T =>
  document.querySelector(sel) as T;

const queryInput = $<HTMLInputElement>("#query");
const resultsEl = $<HTMLElement>("#results");
const statusEl = $<HTMLElement>("#status");
const statsEl = $<HTMLElement>("#stats");
const buildBtn = $<HTMLButtonElement>("#build-btn");
const maxInput = $<HTMLInputElement>("#max-entries");
const kindButtons = Array.from(
  document.querySelectorAll<HTMLButtonElement>(".kind"),
);

let currentKind: Kind = "all";
let debounceTimer: number | undefined;
let hasIndex = false;

// ── 工具函数 ──────────────────────────────────────────────────────────
function setStatus(msg: string, tone: "info" | "warn" | "ok" = "info") {
  statusEl.textContent = msg;
  statusEl.dataset.tone = tone;
}

function basenameOf(path: string): string {
  const i = path.lastIndexOf("/");
  return i >= 0 ? path.slice(i + 1) : path;
}

function dirnameOf(path: string): string {
  const i = path.lastIndexOf("/");
  return i > 0 ? path.slice(0, i) : "/";
}

// 转义 HTML，避免路径中的特殊字符破坏渲染。
function esc(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// 在 basename 上根据 [match_start, match_end) 高亮命中区间（区间是相对整条路径的字节位）。
function highlightBasename(item: MatchItem): string {
  const path = item.path;
  const bnStart = path.lastIndexOf("/") + 1; // 0 若无 '/'
  const bn = basenameOf(path);
  // 命中窗口落在 basename 内的相对区间。
  const s = Math.max(0, item.match_start - bnStart);
  const e = Math.max(s, item.match_end - bnStart);
  if (item.match_end <= item.match_start || s >= bn.length) {
    return esc(bn);
  }
  const a = esc(bn.slice(0, s));
  const b = esc(bn.slice(s, Math.min(e, bn.length)));
  const c = esc(bn.slice(Math.min(e, bn.length)));
  return `${a}<mark>${b}</mark>${c}`;
}

// ── 渲染结果列表 ──────────────────────────────────────────────────────
function renderResults(resp: SearchResponse, query: string) {
  resultsEl.innerHTML = "";
  if (resp.results.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent =
      query.trim().length === 0
        ? "输入关键字开始搜索。"
        : `没有匹配「${query}」的结果。`;
    resultsEl.appendChild(empty);
    return;
  }

  const frag = document.createDocumentFragment();
  for (const item of resp.results) {
    const row = document.createElement("div");
    row.className = "row" + (item.is_dir ? " is-dir" : "");
    row.title = item.path;

    const icon = document.createElement("span");
    icon.className = "row-icon";
    icon.textContent = item.is_dir ? "📁" : "📄";

    const main = document.createElement("div");
    main.className = "row-main";
    const name = document.createElement("div");
    name.className = "row-name";
    name.innerHTML = highlightBasename(item);
    const dir = document.createElement("div");
    dir.className = "row-dir";
    dir.textContent = dirnameOf(item.path);
    main.appendChild(name);
    main.appendChild(dir);

    const score = document.createElement("span");
    score.className = "row-score";
    score.textContent = String(item.score);

    row.appendChild(icon);
    row.appendChild(main);
    row.appendChild(score);

    // 单击：在 Finder 中显示；双击：打开。
    row.addEventListener("dblclick", () => void openTarget(item.path));
    row.addEventListener("click", () => void revealTarget(item.path));

    frag.appendChild(row);
  }
  resultsEl.appendChild(frag);
}

async function openTarget(path: string) {
  try {
    await openPath(path);
  } catch (e) {
    setStatus(`打开失败：${e}`, "warn");
  }
}

async function revealTarget(path: string) {
  try {
    await revealItemInDir(path);
  } catch {
    // 忽略（例如条目已不存在）
  }
}

// ── 搜索 ──────────────────────────────────────────────────────────────
async function doSearch() {
  const query = queryInput.value;
  if (!hasIndex) {
    setStatus("尚无索引，请先点击「建立索引」。", "warn");
    return;
  }
  try {
    const resp = await invoke<SearchResponse>("search", {
      query,
      kind: currentKind,
      limit: 300,
    });
    renderResults(resp, query);
    statsEl.textContent = `命中 ${resp.total} 条 · 后端耗时 ${resp.elapsed_ms} ms`;
  } catch (e) {
    setStatus(`搜索出错：${e}`, "warn");
  }
}

function scheduleSearch() {
  window.clearTimeout(debounceTimer);
  debounceTimer = window.setTimeout(() => void doSearch(), 90);
}

// ── 建索引 ────────────────────────────────────────────────────────────
async function doBuildIndex() {
  const max = parseInt(maxInput.value || "0", 10);
  buildBtn.disabled = true;
  buildBtn.textContent = "建立中…";
  setStatus("正在遍历文件系统并建立二进制索引…", "info");
  try {
    // roots 为空 → 后端默认索引 $HOME。
    const res = await invoke<IndexResult>("build_index", {
      roots: [] as string[],
      maxEntries: Number.isFinite(max) && max > 0 ? max : null,
      outPath: null,
    });
    hasIndex = true;
    setStatus(
      `索引就绪：${res.entry_count.toLocaleString()} 条目 · 文件 ${(
        res.file_size / 1_048_576
      ).toFixed(1)} MB · 建索引 ${res.elapsed_ms} ms`,
      "ok",
    );
    await doSearch();
  } catch (e) {
    setStatus(`建立索引失败：${e}`, "warn");
  } finally {
    buildBtn.disabled = false;
    buildBtn.textContent = "重建索引";
  }
}

// ── 启动：探测已有索引 ────────────────────────────────────────────────
async function init() {
  // 类型过滤按钮。
  for (const btn of kindButtons) {
    btn.addEventListener("click", () => {
      kindButtons.forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      currentKind = (btn.dataset.kind as Kind) ?? "all";
      void doSearch();
    });
  }

  queryInput.addEventListener("input", scheduleSearch);
  buildBtn.addEventListener("click", () => void doBuildIndex());

  // 尝试加载默认路径已有的索引（若上次已建过）。
  try {
    const status = await invoke<IndexStatus>("load_index", {
      indexPath: null,
    });
    if (status.loaded && status.entry_count > 0) {
      hasIndex = true;
      buildBtn.textContent = "重建索引";
      setStatus(
        `已加载现有索引：${status.entry_count.toLocaleString()} 条目 · ${status.index_path}`,
        "ok",
      );
      queryInput.focus();
      await doSearch();
      return;
    }
  } catch {
    // 没有现成索引，属正常情况。
  }
  setStatus("尚无索引。点击「建立索引」扫描你的主目录后即可秒搜。", "info");
  queryInput.focus();
}

void init();

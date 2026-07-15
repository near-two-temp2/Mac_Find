// Frontend controller: wires the search box + result list to the Rust hybrid
// engine, debounces input, and renders engine/status info.

import * as api from "./api";
import type { EngineStatus, Hit, SearchOptions, SearchResponse } from "./types";

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const queryEl = $<HTMLInputElement>("query");
const filesEl = $<HTMLInputElement>("opt-files");
const dirsEl = $<HTMLInputElement>("opt-dirs");
const liveEl = $<HTMLInputElement>("opt-live");
const rebuildEl = $<HTMLButtonElement>("rebuild");
const statusEl = $<HTMLElement>("status");
const resultsEl = $<HTMLElement>("results");
const footerEl = $<HTMLElement>("footer");

let debounceTimer: number | undefined;
let searchSeq = 0; // guards against out-of-order async responses

function currentOptions(): SearchOptions {
  return {
    filesOnly: filesEl.checked,
    dirsOnly: dirsEl.checked,
    limit: 1000,
  };
}

async function refreshStatus(): Promise<void> {
  try {
    const s: EngineStatus = await api.engineStatus();
    const engine = s.indexReady
      ? `index (${s.indexEntries.toLocaleString()} entries)`
      : "searchfs fallback";
    const fb = s.searchfsAvailable ? "available" : "unavailable";
    statusEl.textContent = `Engine: ${engine} · searchfs fallback ${fb}`;
    statusEl.classList.toggle("warn", !s.indexReady);
  } catch (e) {
    statusEl.textContent = `Status error: ${String(e)}`;
    statusEl.classList.add("warn");
  }
}

function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"']/g,
    (c) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[c] as string,
  );
}

function render(resp: SearchResponse): void {
  if (resp.hits.length === 0) {
    resultsEl.innerHTML = `<div class="empty">No matches.</div>`;
  } else {
    const rows = resp.hits
      .map((h: Hit, i: number) => {
        const icon = h.isDir ? "📁" : "📄";
        const dir = h.path.slice(0, Math.max(0, h.path.length - h.name.length));
        return `<div class="row" data-idx="${i}" data-path="${escapeHtml(
          h.path,
        )}" tabindex="0">
          <span class="icon">${icon}</span>
          <span class="name">${escapeHtml(h.name)}</span>
          <span class="dir">${escapeHtml(dir)}</span>
        </div>`;
      })
      .join("");
    resultsEl.innerHTML = rows;
  }
  footerEl.textContent = `${resp.hits.length} shown · ${resp.scanned.toLocaleString()} scanned · ${
    resp.elapsedMs
  } ms · via ${resp.engine}`;
}

async function runSearch(): Promise<void> {
  const q = queryEl.value.trim();
  const seq = ++searchSeq;
  if (q.length === 0) {
    resultsEl.innerHTML = `<div class="empty">Type to search.</div>`;
    footerEl.textContent = "";
    return;
  }
  try {
    const opts = currentOptions();
    const resp = liveEl.checked
      ? await api.searchLive(q, opts)
      : await api.search(q, opts);
    if (seq !== searchSeq) return; // a newer search superseded this one
    render(resp);
  } catch (e) {
    if (seq !== searchSeq) return;
    resultsEl.innerHTML = `<div class="empty error">${escapeHtml(String(e))}</div>`;
    footerEl.textContent = "";
  }
}

function scheduleSearch(): void {
  window.clearTimeout(debounceTimer);
  debounceTimer = window.setTimeout(runSearch, 120);
}

// ---- event wiring ---------------------------------------------------------

queryEl.addEventListener("input", scheduleSearch);
[filesEl, dirsEl, liveEl].forEach((el) =>
  el.addEventListener("change", () => {
    // Files-only and dirs-only are mutually exclusive.
    if (el === filesEl && filesEl.checked) dirsEl.checked = false;
    if (el === dirsEl && dirsEl.checked) filesEl.checked = false;
    runSearch();
  }),
);

// Row interactions: double-click opens, right-click reveals in Finder,
// single-click just selects.
resultsEl.addEventListener("dblclick", (ev) => {
  const row = (ev.target as HTMLElement).closest(".row") as HTMLElement | null;
  if (row) api.openPath(row.dataset.path!).catch(console.error);
});

resultsEl.addEventListener("contextmenu", (ev) => {
  const row = (ev.target as HTMLElement).closest(".row") as HTMLElement | null;
  if (row) {
    ev.preventDefault();
    api.revealInFinder(row.dataset.path!).catch(console.error);
  }
});

resultsEl.addEventListener("keydown", (ev) => {
  const row = (ev.target as HTMLElement).closest(".row") as HTMLElement | null;
  if (!row) return;
  if (ev.key === "Enter") api.openPath(row.dataset.path!).catch(console.error);
  if (ev.key === " ") {
    ev.preventDefault();
    api.revealInFinder(row.dataset.path!).catch(console.error);
  }
});

rebuildEl.addEventListener("click", async () => {
  rebuildEl.disabled = true;
  const prev = rebuildEl.textContent;
  rebuildEl.textContent = "Building…";
  statusEl.textContent = "Building index (this may take a while)…";
  try {
    const n = await api.rebuildIndex();
    statusEl.textContent = `Indexed ${n.toLocaleString()} entries.`;
    await refreshStatus();
    await runSearch();
  } catch (e) {
    statusEl.textContent = `Index build failed: ${String(e)}`;
    statusEl.classList.add("warn");
  } finally {
    rebuildEl.disabled = false;
    rebuildEl.textContent = prev;
  }
});

// ---- boot -----------------------------------------------------------------

window.addEventListener("DOMContentLoaded", () => {
  queryEl.focus();
  refreshStatus();
});

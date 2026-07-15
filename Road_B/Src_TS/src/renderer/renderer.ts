// renderer.ts — the search UI. Talks to the main process only through the
// `window.macfind` bridge exposed by preload.ts.

interface SearchHit {
  path: string;
  score: number;
  isDir: boolean;
  start: number;
  end: number;
}

interface MacFindBridge {
  indexStatus(): Promise<{ ready: boolean; meta: any; cacheDir: string }>;
  buildIndex(roots?: string[], maxEntries?: number): Promise<{ ok: boolean; meta: any }>;
  search(
    query: string,
    opts?: Record<string, unknown>
  ): Promise<{ hits: SearchHit[]; ms?: number; indexed?: number; error?: string }>;
  onProgress(cb: (count: number) => void): () => void;
}

// This file is loaded as a classic <script>, so avoid module syntax entirely
// and read the preload bridge off the global window object.
const macfind: MacFindBridge = (window as unknown as { macfind: MacFindBridge })
  .macfind;

const $ = (id: string) => document.getElementById(id)!;
const qInput = $("q") as HTMLInputElement;
const filesOnly = $("filesOnly") as HTMLInputElement;
const dirsOnly = $("dirsOnly") as HTMLInputElement;
const rebuildBtn = $("rebuild") as HTMLButtonElement;
const statusEl = $("status");
const resultsEl = $("results");
const footEl = $("foot");

let debounce: number | undefined;

function setStatus(text: string): void {
  statusEl.textContent = text;
}

async function refreshStatus(): Promise<void> {
  const s = await macfind.indexStatus();
  if (s.ready && s.meta) {
    setStatus(
      `Index ready — ${s.meta.entryCount.toLocaleString()} entries · built ${new Date(
        s.meta.builtAt
      ).toLocaleString()}`
    );
  } else {
    setStatus('No index yet. Click "Build index" to scan your home folder.');
  }
}

// Escape text for safe insertion, highlighting the [start,end) matched slice.
function renderPath(hit: SearchHit): string {
  const p = hit.path;
  const s = Math.max(0, Math.min(hit.start, p.length));
  const e = Math.max(s, Math.min(hit.end, p.length));
  const esc = (t: string) =>
    t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  if (e <= s) return esc(p);
  return `${esc(p.slice(0, s))}<span class="m">${esc(p.slice(s, e))}</span>${esc(
    p.slice(e)
  )}`;
}

function renderHits(hits: SearchHit[], ms?: number, indexed?: number): void {
  if (hits.length === 0) {
    resultsEl.innerHTML = '<div class="empty">No matches.</div>';
  } else {
    const rows = hits
      .map(
        (h) =>
          `<div class="row"><span class="kind">${
            h.isDir ? "📁" : "📄"
          }</span><span class="score">${h.score}</span><span class="path">${renderPath(
            h
          )}</span></div>`
      )
      .join("");
    resultsEl.innerHTML = rows;
  }
  footEl.textContent =
    ms !== undefined
      ? `${hits.length} hits · ${ms} ms · ${indexed?.toLocaleString() ?? "?"} indexed`
      : "";
}

async function runSearch(): Promise<void> {
  const query = qInput.value;
  const res = await macfind.search(query, {
    limit: 300,
    filesOnly: filesOnly.checked,
    dirsOnly: dirsOnly.checked,
  });
  if (res.error) {
    resultsEl.innerHTML = `<div class="empty">${res.error}. Build an index first.</div>`;
    footEl.textContent = "";
    return;
  }
  renderHits(res.hits, res.ms, res.indexed);
}

function scheduleSearch(): void {
  if (debounce) window.clearTimeout(debounce);
  debounce = window.setTimeout(runSearch, 60);
}

qInput.addEventListener("input", scheduleSearch);
filesOnly.addEventListener("change", () => {
  if (dirsOnly.checked && filesOnly.checked) dirsOnly.checked = false;
  runSearch();
});
dirsOnly.addEventListener("change", () => {
  if (dirsOnly.checked && filesOnly.checked) filesOnly.checked = false;
  runSearch();
});

rebuildBtn.addEventListener("click", async () => {
  rebuildBtn.disabled = true;
  const stop = macfind.onProgress((c) => setStatus(`Indexing… ${c.toLocaleString()} entries`));
  try {
    setStatus("Indexing… scanning home folder");
    await macfind.buildIndex();
    await refreshStatus();
    await runSearch();
  } catch (e) {
    setStatus(`Index build failed: ${(e as Error).message}`);
  } finally {
    stop();
    rebuildBtn.disabled = false;
  }
});

// Boot.
refreshStatus();
qInput.focus();

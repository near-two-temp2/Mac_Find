// Road A (Tauri) front end.
//
// Thin TypeScript UI over the Rust `search_files` command. All filesystem work
// happens in Rust via searchfs(); this file just collects options, debounces
// input, invokes the command, and renders the result list.

import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

/** Mirrors `searchfs::SearchQuery` (serde camelCase). */
interface SearchQuery {
  term: string;
  dirsOnly: boolean;
  filesOnly: boolean;
  caseSensitive: boolean;
  exactMatch: boolean;
  limit: number;
}

/** Mirrors `searchfs::SearchHit`. */
interface SearchHit {
  path: string;
  name: string;
  isDir: boolean;
}

/** Mirrors `searchfs::SearchResult`. */
interface SearchResult {
  hits: SearchHit[];
  volumesSearched: string[];
  notes: string[];
  truncated: boolean;
}

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const termEl = $<HTMLInputElement>("term");
const goEl = $<HTMLButtonElement>("go");
const caseEl = $<HTMLInputElement>("case");
const exactEl = $<HTMLInputElement>("exact");
const limitEl = $<HTMLInputElement>("limit");
const resultsEl = $<HTMLDivElement>("results");
const statusEl = $<HTMLDivElement>("status");

/** Read the currently selected file/dir/all radio. */
function selectedKind(): "all" | "files" | "dirs" {
  const checked = document.querySelector<HTMLInputElement>(
    'input[name="kind"]:checked',
  );
  return (checked?.value as "all" | "files" | "dirs") ?? "all";
}

function buildQuery(): SearchQuery {
  const kind = selectedKind();
  const limitRaw = parseInt(limitEl.value, 10);
  return {
    term: termEl.value.trim(),
    filesOnly: kind === "files",
    dirsOnly: kind === "dirs",
    caseSensitive: caseEl.checked,
    exactMatch: exactEl.checked,
    limit: Number.isFinite(limitRaw) && limitRaw >= 0 ? limitRaw : 1000,
  };
}

let searchSeq = 0; // guards against out-of-order async responses

async function runSearch(): Promise<void> {
  const query = buildQuery();
  const seq = ++searchSeq;

  if (query.term.length === 0) {
    resultsEl.replaceChildren();
    statusEl.textContent = "Type something to search.";
    return;
  }

  statusEl.textContent = `Searching for “${query.term}”…`;
  resultsEl.setAttribute("aria-busy", "true");

  const started = performance.now();
  try {
    const res = await invoke<SearchResult>("search_files", { query });
    if (seq !== searchSeq) return; // a newer search superseded this one
    render(res, query, performance.now() - started);
  } catch (err) {
    if (seq !== searchSeq) return;
    resultsEl.replaceChildren();
    statusEl.textContent = `Error: ${String(err)}`;
  } finally {
    resultsEl.removeAttribute("aria-busy");
  }
}

function render(res: SearchResult, query: SearchQuery, ms: number): void {
  resultsEl.replaceChildren();

  const frag = document.createDocumentFragment();
  for (const hit of res.hits) {
    const row = document.createElement("div");
    row.className = "row";
    row.setAttribute("role", "option");
    row.title = hit.path;

    const icon = document.createElement("span");
    icon.className = "icon";
    icon.textContent = hit.isDir ? "📁" : "📄";

    const name = document.createElement("span");
    name.className = "name";
    name.append(highlight(hit.name, query));

    const dir = document.createElement("span");
    dir.className = "dir";
    dir.textContent = parentOf(hit.path);

    row.append(icon, name, dir);
    frag.append(row);
  }
  resultsEl.append(frag);

  const parts = [`${res.hits.length} result${res.hits.length === 1 ? "" : "s"}`];
  parts.push(`${ms.toFixed(0)} ms`);
  if (res.volumesSearched.length) {
    parts.push(`vols: ${res.volumesSearched.join(", ")}`);
  }
  if (res.truncated) parts.push("(truncated — raise limit)");
  if (res.notes.length) parts.push(`⚠ ${res.notes.join("; ")}`);
  statusEl.textContent = parts.join("  ·  ");
}

/** Directory portion of an absolute path (everything before the last "/"). */
function parentOf(path: string): string {
  const i = path.lastIndexOf("/");
  return i <= 0 ? "/" : path.slice(0, i);
}

/** Emphasize the matched substring inside the filename. */
function highlight(name: string, query: SearchQuery): DocumentFragment {
  const frag = document.createDocumentFragment();
  const term = query.term;
  if (!term) {
    frag.append(name);
    return frag;
  }
  const hay = query.caseSensitive ? name : name.toLowerCase();
  const needle = query.caseSensitive ? term : term.toLowerCase();
  const idx = hay.indexOf(needle);
  if (idx < 0) {
    frag.append(name);
    return frag;
  }
  frag.append(name.slice(0, idx));
  const mark = document.createElement("mark");
  mark.textContent = name.slice(idx, idx + term.length);
  frag.append(mark);
  frag.append(name.slice(idx + term.length));
  return frag;
}

// --- wiring -------------------------------------------------------------

let debounce: number | undefined;
termEl.addEventListener("input", () => {
  window.clearTimeout(debounce);
  debounce = window.setTimeout(runSearch, 250);
});
termEl.addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    window.clearTimeout(debounce);
    void runSearch();
  }
});
goEl.addEventListener("click", () => void runSearch());
for (const el of [caseEl, exactEl, limitEl]) {
  el.addEventListener("change", () => void runSearch());
}
for (const el of document.querySelectorAll('input[name="kind"]')) {
  el.addEventListener("change", () => void runSearch());
}

// Show engine info in the status bar on load; confirms the bridge is live.
invoke<Record<string, unknown>>("engine_info")
  .then((info) => {
    statusEl.textContent = `Ready — engine: ${info.engine} · road ${info.road} · ${info.stack}`;
  })
  .catch(() => {
    statusEl.textContent = "Ready.";
  });

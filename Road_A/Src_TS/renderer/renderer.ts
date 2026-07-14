/*
 * renderer.ts — UI logic for the search window.
 *
 * Talks to the main process only through the preload-exposed window.macFind
 * bridge. Debounces keystrokes, collects options, and renders results.
 */

interface SearchOptions {
  dirsOnly?: boolean;
  filesOnly?: boolean;
  exactMatch?: boolean;
  caseSensitive?: boolean;
  skipInvisibles?: boolean;
  limit?: number;
}

interface SearchResponse {
  ok: boolean;
  results?: string[];
  error?: string;
}

interface MacFindApi {
  search(term: string, options: SearchOptions): Promise<SearchResponse>;
  engineAvailable(): Promise<boolean>;
}

declare global {
  interface Window {
    macFind: MacFindApi;
  }
}

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const searchInput = $<HTMLInputElement>('search');
const resultsEl = $<HTMLUListElement>('results');
const emptyEl = $<HTMLDivElement>('empty');
const statusEl = $<HTMLSpanElement>('status');
const spinnerEl = $<HTMLSpanElement>('spinner');
const bannerEl = $<HTMLDivElement>('banner');
const caseSensitiveEl = $<HTMLInputElement>('caseSensitive');
const exactEl = $<HTMLInputElement>('exact');
const skipInvisiblesEl = $<HTMLInputElement>('skipInvisibles');
const limitEl = $<HTMLInputElement>('limit');

let debounceTimer: number | undefined;
let requestSeq = 0; // guards against out-of-order responses

function currentOptions(): SearchOptions {
  const kind = (document.querySelector('input[name="kind"]:checked') as HTMLInputElement)?.value;
  const limit = parseInt(limitEl.value, 10);
  return {
    dirsOnly: kind === 'dirs',
    filesOnly: kind === 'files',
    exactMatch: exactEl.checked,
    caseSensitive: caseSensitiveEl.checked,
    skipInvisibles: skipInvisiblesEl.checked,
    limit: Number.isFinite(limit) && limit > 0 ? limit : 0,
  };
}

function basename(p: string): string {
  const i = p.lastIndexOf('/');
  return i >= 0 ? p.slice(i + 1) : p;
}

function render(results: string[]): void {
  resultsEl.textContent = '';
  if (results.length === 0) {
    emptyEl.textContent = 'No matches';
    emptyEl.classList.remove('hidden');
    return;
  }
  emptyEl.classList.add('hidden');

  // Build off-DOM for speed with large result sets.
  const frag = document.createDocumentFragment();
  for (const full of results) {
    const li = document.createElement('li');
    const name = document.createElement('span');
    name.className = 'name';
    name.textContent = basename(full);
    const path = document.createElement('span');
    path.className = 'path';
    path.textContent = full;
    path.title = full;
    li.appendChild(name);
    li.appendChild(path);
    frag.appendChild(li);
  }
  resultsEl.appendChild(frag);
}

async function runSearch(): Promise<void> {
  const term = searchInput.value.trim();
  const seq = ++requestSeq;

  if (term.length === 0) {
    render([]);
    emptyEl.textContent = 'Type to search…';
    emptyEl.classList.remove('hidden');
    statusEl.textContent = 'Ready';
    return;
  }

  spinnerEl.classList.remove('hidden');
  statusEl.textContent = 'Searching…';

  const started = performance.now();
  const resp = await window.macFind.search(term, currentOptions());

  // Drop stale responses.
  if (seq !== requestSeq) return;
  spinnerEl.classList.add('hidden');

  if (!resp.ok) {
    statusEl.textContent = `Error: ${resp.error ?? 'unknown'}`;
    render([]);
    emptyEl.textContent = 'Search failed';
    emptyEl.classList.remove('hidden');
    return;
  }

  const results = resp.results ?? [];
  const ms = Math.round(performance.now() - started);
  render(results);
  statusEl.textContent = `${results.length} result(s) in ${ms} ms`;
}

function scheduleSearch(): void {
  window.clearTimeout(debounceTimer);
  debounceTimer = window.setTimeout(runSearch, 180);
}

searchInput.addEventListener('input', scheduleSearch);
document.querySelectorAll('input[name="kind"]').forEach((el) =>
  el.addEventListener('change', runSearch),
);
[caseSensitiveEl, exactEl, skipInvisiblesEl, limitEl].forEach((el) =>
  el.addEventListener('change', runSearch),
);

// Warn early if the native engine failed to load (missing addon / permissions).
window.macFind.engineAvailable().then((available) => {
  if (!available) {
    bannerEl.textContent =
      'Native searchfs engine unavailable. Build the addon on macOS (npm run build:addon) ' +
      'and grant Full Disk Access to search system volumes.';
    bannerEl.classList.remove('hidden');
  }
});

searchInput.focus();

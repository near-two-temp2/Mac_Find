/*
 * renderer.ts — UI logic for the search window.
 *
 * A plain browser script (no import/export) so tsc emits <script>-ready JS.
 * Talks only to window.macfind (the preload bridge). Debounced search box,
 * result list, "Reveal in Finder" / "Open", a mode badge (index vs searchfs
 * fallback), and a "Build index" button. Types come from renderer/global.d.ts.
 */

function q<T extends HTMLElement>(sel: string): T {
  return document.querySelector(sel) as T;
}

const input = q<HTMLInputElement>('#q');
const list = q<HTMLUListElement>('#results');
const badge = q<HTMLSpanElement>('#mode');
const statusLine = q<HTMLDivElement>('#status');
const filterSel = q<HTMLSelectElement>('#filter');
const reindexBtn = q<HTMLButtonElement>('#reindex');

let debounce: ReturnType<typeof setTimeout> | null = null;
let seq = 0;

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) =>
    c === '&'
      ? '&amp;'
      : c === '<'
        ? '&lt;'
        : c === '>'
          ? '&gt;'
          : c === '"'
            ? '&quot;'
            : '&#39;'
  );
}

function basename(p: string): string {
  const i = p.lastIndexOf('/');
  return i < 0 ? p : p.slice(i + 1);
}

function render(res: MacFindSearchResult): void {
  badge.textContent = res.mode;
  badge.className = `badge badge-${res.mode}`;
  statusLine.textContent = `${res.results.length} results · ${res.tookMs} ms · ${res.note ?? ''}`;

  list.innerHTML = '';
  const frag = document.createDocumentFragment();
  for (const hit of res.results) {
    const li = document.createElement('li');
    li.className = 'row';
    const name = basename(hit.path);
    const dir = hit.path.slice(0, hit.path.length - name.length);
    li.innerHTML =
      `<span class="icon">${hit.isDir ? '📁' : '📄'}</span>` +
      `<span class="name">${esc(name)}</span>` +
      `<span class="dir">${esc(dir)}</span>` +
      `<span class="actions">` +
      `<button class="reveal" title="Reveal in Finder">Finder</button>` +
      `<button class="open" title="Open">Open</button>` +
      `</span>`;
    li.querySelector('.reveal')!.addEventListener('click', (e) => {
      e.stopPropagation();
      window.macfind.reveal(hit.path);
    });
    li.querySelector('.open')!.addEventListener('click', (e) => {
      e.stopPropagation();
      window.macfind.open(hit.path);
    });
    li.addEventListener('dblclick', () => window.macfind.reveal(hit.path));
    frag.appendChild(li);
  }
  list.appendChild(frag);
}

async function runSearch(): Promise<void> {
  const pattern = input.value.trim();
  const mySeq = ++seq;
  if (!pattern) {
    list.innerHTML = '';
    statusLine.textContent = 'Type to search.';
    return;
  }
  const filter = filterSel.value; // 'all' | 'files' | 'dirs'
  const res = await window.macfind.search(pattern, {
    limit: 200,
    filesOnly: filter === 'files',
    dirsOnly: filter === 'dirs',
  });
  if (mySeq !== seq) return; // a newer query superseded this one
  render(res);
}

function scheduleSearch(): void {
  if (debounce) clearTimeout(debounce);
  debounce = setTimeout(runSearch, 120);
}

async function refreshStatus(): Promise<void> {
  const s = await window.macfind.status();
  const parts = [
    s.hasIndex ? `index: ${s.entries} entries` : 'index: none',
    s.fallbackReady ? 'searchfs: ready' : 'searchfs: unavailable',
  ];
  statusLine.textContent = parts.join(' · ');
}

input.addEventListener('input', scheduleSearch);
filterSel.addEventListener('change', runSearch);

reindexBtn.addEventListener('click', async () => {
  reindexBtn.disabled = true;
  reindexBtn.textContent = 'Indexing…';
  try {
    const count = await window.macfind.reindex();
    statusLine.textContent = `Built index: ${count} entries`;
  } finally {
    reindexBtn.disabled = false;
    reindexBtn.textContent = 'Build index';
    await refreshStatus();
  }
});

refreshStatus();
input.focus();

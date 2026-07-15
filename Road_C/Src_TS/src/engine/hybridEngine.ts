/*
 * hybridEngine.ts — Road_C hybrid search orchestrator (the "complete" version).
 *
 * Strategy (open-source-analysis.md §5.4):
 *   PRIMARY   : self-built binary index (typed arrays + bitmask prefilter + fzf
 *               scoring), sharded across worker_threads for parallelism.
 *   FALLBACK  : native searchfs() live search, used when the index is
 *               missing / corrupt / empty, or when the pattern is better served
 *               by a live catalog scan.
 *
 * The engine reports which path served each query so the UI can display the
 * active mode ("index" vs "searchfs fallback").
 */

import { Worker } from 'worker_threads';
import * as os from 'os';
import * as path from 'path';
import {
  loadIndex,
  writeIndex,
  defaultIndexPath,
} from './indexStore';
import {
  materialize,
  queryIndex,
  type QueryOptions,
  type QueryResult,
  type ScoredHit,
} from './query';
import type { IndexView } from './binaryIndex';
import { scanFilesystem } from './scanner';
import {
  searchfsAvailable,
  searchfsSearch,
  searchfsLoadError,
} from './searchfsFallback';

export type EngineMode = 'index' | 'searchfs' | 'none';

export interface HybridSearchResult {
  mode: EngineMode;
  results: QueryResult[];
  tookMs: number;
  note?: string;
}

export interface HybridConfig {
  indexPath?: string;
  workerScript?: string; // path to compiled searchWorker.js
  workerCount?: number;
  fallbackLimit?: number;
}

interface Shard {
  worker: Worker;
  start: number;
  end: number;
}

let nextQueryId = 1;

export class HybridEngine {
  private view: IndexView | null = null;
  private shards: Shard[] = [];
  private readonly indexPath: string;
  private readonly workerScript: string | null;
  private readonly workerCount: number;
  private readonly fallbackLimit: number;
  private ready = false;

  constructor(cfg: HybridConfig = {}) {
    this.indexPath = cfg.indexPath ?? defaultIndexPath();
    this.workerScript = cfg.workerScript ?? null;
    this.workerCount = cfg.workerCount ?? Math.max(1, Math.min(4, os.cpus().length));
    this.fallbackLimit = cfg.fallbackLimit ?? 1000;
  }

  /** Load the index (if present) and spin up worker shards. Safe to call twice. */
  async init(): Promise<void> {
    if (this.ready) return;
    this.view = loadIndex(this.indexPath);
    if (this.view && this.workerScript) {
      await this.spawnWorkers();
    }
    this.ready = true;
  }

  /** Whether a usable index is currently loaded. */
  hasIndex(): boolean {
    return this.view !== null && this.view.entryCount > 0;
  }

  indexEntryCount(): number {
    return this.view ? this.view.entryCount : 0;
  }

  fallbackReady(): boolean {
    return searchfsAvailable();
  }

  fallbackError(): string | null {
    return searchfsLoadError();
  }

  private async spawnWorkers(): Promise<void> {
    if (!this.view || !this.workerScript) return;
    const n = this.view.entryCount;
    const per = Math.ceil(n / this.workerCount);
    const readyPromises: Promise<void>[] = [];

    for (let w = 0; w < this.workerCount; w++) {
      const start = w * per;
      const end = Math.min(n, start + per);
      if (start >= end) break;

      const worker = new Worker(this.workerScript);
      // Clone the buffer for this worker (transfer would detach the shared one).
      const copy = this.view.buffer.slice(0);
      const shard: Shard = { worker, start, end };
      this.shards.push(shard);

      const ready = new Promise<void>((resolve) => {
        const onReady = (msg: any) => {
          if (msg && msg.type === 'ready') {
            worker.off('message', onReady);
            resolve();
          }
        };
        worker.on('message', onReady);
      });
      worker.postMessage({ type: 'init', buffer: copy }, [copy]);
      readyPromises.push(ready);
    }
    await Promise.all(readyPromises);
  }

  /**
   * Rebuild the index from `roots` and persist it, then reload. Returns entry
   * count. Tears down and respawns workers around the new index.
   */
  async rebuildIndex(roots: string[], maxEntries?: number): Promise<number> {
    const entries = scanFilesystem({ roots, maxEntries });
    writeIndex(entries, this.indexPath);
    await this.teardownWorkers();
    this.view = loadIndex(this.indexPath);
    if (this.view && this.workerScript) {
      await this.spawnWorkers();
    }
    return this.view ? this.view.entryCount : 0;
  }

  private async teardownWorkers(): Promise<void> {
    await Promise.all(this.shards.map((s) => s.worker.terminate()));
    this.shards = [];
  }

  /**
   * Run a query. Uses the index when available; otherwise (or on empty results
   * with a viable fallback) degrades to searchfs().
   */
  async search(pattern: string, opts: QueryOptions = {}): Promise<HybridSearchResult> {
    const t0 = Date.now();

    if (this.hasIndex()) {
      const results = await this.searchViaIndex(pattern, opts);
      return {
        mode: 'index',
        results,
        tookMs: Date.now() - t0,
        note: `index: ${this.indexEntryCount()} entries`,
      };
    }

    // No index -> searchfs fallback.
    if (searchfsAvailable()) {
      const paths = searchfsSearch(pattern, {
        dirsOnly: opts.dirsOnly,
        filesOnly: opts.filesOnly,
        limit: opts.limit ?? this.fallbackLimit,
      });
      const results: QueryResult[] = paths.map((p) => ({
        path: p,
        isDir: false, // searchfs() addon doesn't return type; UI can stat lazily
        score: 0,
      }));
      return {
        mode: 'searchfs',
        results,
        tookMs: Date.now() - t0,
        note: 'index missing — live searchfs() fallback',
      };
    }

    return {
      mode: 'none',
      results: [],
      tookMs: Date.now() - t0,
      note: `no index and searchfs unavailable (${searchfsLoadError() ?? 'n/a'})`,
    };
  }

  private async searchViaIndex(
    pattern: string,
    opts: QueryOptions
  ): Promise<QueryResult[]> {
    const view = this.view!;
    const limit = opts.limit ?? 200;

    // No workers (or index tiny) -> single-threaded path.
    if (this.shards.length === 0) {
      return queryIndex(view, pattern, opts);
    }

    // Fan out to workers, each returns its slice's top-K, then merge.
    const id = nextQueryId++;
    const perShardOpts: QueryOptions = { ...opts, limit };

    const shardHits = await Promise.all(
      this.shards.map(
        (shard) =>
          new Promise<ScoredHit[]>((resolve) => {
            const onMsg = (msg: any) => {
              if (msg && msg.type === 'result' && msg.id === id) {
                shard.worker.off('message', onMsg);
                resolve(msg.hits as ScoredHit[]);
              }
            };
            shard.worker.on('message', onMsg);
            shard.worker.postMessage({
              type: 'query',
              id,
              pattern,
              start: shard.start,
              end: shard.end,
              opts: perShardOpts,
            });
          })
      )
    );

    // Merge, global top-K, materialize with original-case paths.
    const merged: ScoredHit[] = ([] as ScoredHit[]).concat(...shardHits);
    merged.sort((a, b) => b.score - a.score);
    if (merged.length > limit) merged.length = limit;
    return materialize(view, merged);
  }

  async dispose(): Promise<void> {
    await this.teardownWorkers();
    this.ready = false;
  }
}

/** Default index roots: user home + /Applications (keeps first scan bounded). */
export function defaultRoots(): string[] {
  return [os.homedir(), '/Applications'];
}

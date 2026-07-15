/*
 * searchWorker.ts — worker_threads entry point for parallel Phase 1/2 scoring.
 *
 * The coordinator (hybridEngine.ts) transfers the index ArrayBuffer to each
 * worker once (SharedArrayBuffer-style zero-copy isn't used so the buffer is
 * cloned per worker; for the sizes we target this is acceptable and keeps the
 * code simple). Each worker then scores its assigned slice per query.
 *
 * Protocol:
 *   main -> worker  { type: 'init', buffer }          (transfer buffer once)
 *   main -> worker  { type: 'query', id, pattern, start, end, opts }
 *   worker -> main  { type: 'result', id, hits }      (ScoredHit[])
 */

import { parentPort } from 'worker_threads';
import { loadIndexView, type IndexView } from './binaryIndex';
import { scoreSlice, type QueryOptions } from './query';

let view: IndexView | null = null;

if (!parentPort) {
  throw new Error('searchWorker must run as a worker_thread');
}

parentPort.on('message', (msg: any) => {
  if (msg.type === 'init') {
    view = loadIndexView(msg.buffer as ArrayBuffer);
    parentPort!.postMessage({ type: 'ready' });
    return;
  }

  if (msg.type === 'query') {
    if (!view) {
      parentPort!.postMessage({ type: 'result', id: msg.id, hits: [] });
      return;
    }
    const patternLower = (msg.pattern as string).toLowerCase();
    const hits = scoreSlice(
      view,
      patternLower,
      msg.start as number,
      msg.end as number,
      msg.opts as QueryOptions
    );
    parentPort!.postMessage({ type: 'result', id: msg.id, hits });
  }
});

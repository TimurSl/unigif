// Live updates: one EventSource on /api/v1/events turns server-side changes (an upload through the API, a GIF
// that finished processing, a tag edit from another tab) into the same notifications the UI already uses for
// its own edits, so open lists update without a reload.
import { api, ApiError } from './api';
import { emitGif, emitResync, refreshStats, refreshTopTags } from './app.svelte';

let source: EventSource | null = null;
let wasDown = false;

/** Events about these ids are skipped for a few seconds: this tab just made the change and already shows it. */
const quiet = new Map<string, number>();
const QUIET_MS = 4000;

export function ignoreEventsFor(ids: Iterable<string>) {
  const until = Date.now() + QUIET_MS;
  for (const id of ids) quiet.set(id, until);
}

function muted(id: string): boolean {
  const until = quiet.get(id);
  if (until === undefined) return false;
  if (until < Date.now()) {
    quiet.delete(id);
    return false;
  }
  return true;
}

type Kind = 'gif.added' | 'gif.ready' | 'gif.failed' | 'gif.tagged' | 'gif.deleted';
const pending = new Map<string, Kind>();
let flushTimer: ReturnType<typeof setTimeout> | undefined;
/// More changed GIFs than this in one burst: reload the lists instead of fetching each one.
const BURST = 40;

function queue(kind: Kind, id: string) {
  if (muted(id)) return;
  // Several events for one GIF in a burst collapse into the last one, except that "added" is not downgraded
  // (the list needs to insert it, and fetching it gives its latest state anyway).
  const prev = pending.get(id);
  pending.set(id, kind !== 'gif.added' && prev === 'gif.added' ? prev : kind);
  clearTimeout(flushTimer);
  flushTimer = setTimeout(flush, 150);
}

async function flush() {
  const batch = [...pending];
  pending.clear();
  if (batch.length === 0) return;
  void refreshStats();
  refreshTopTags(true);
  if (batch.length > BURST) {
    emitResync();
    return;
  }
  await Promise.all(
    batch.map(async ([id, kind]) => {
      if (kind === 'gif.deleted') {
        // The server only says which GIF; every list that shows it needs just the id.
        emitGif('removed', { id } as never);
        return;
      }
      try {
        const gif = await api.getGif(id);
        emitGif(kind === 'gif.added' ? 'added' : 'changed', gif);
      } catch (e) {
        // Gone or hidden again by the time we asked.
        if (e instanceof ApiError && (e.status === 404 || e.status === 403)) emitGif('removed', { id } as never);
      }
    }),
  );
}

export function startLive() {
  if (source || typeof EventSource === 'undefined') return;
  const es = new EventSource('/api/v1/events');
  source = es;
  for (const kind of ['gif.added', 'gif.ready', 'gif.failed', 'gif.tagged', 'gif.deleted'] as const) {
    es.addEventListener(kind, (e) => {
      try {
        const { id } = JSON.parse((e as MessageEvent<string>).data) as { id: string };
        queue(kind, id);
      } catch {
        /* a malformed event is ignored */
      }
    });
  }
  es.addEventListener('resync', () => {
    emitResync();
    void refreshStats();
    refreshTopTags(true);
  });
  es.onerror = () => {
    wasDown = true;
  };
  es.onopen = () => {
    // Events may have been missed while the connection was down.
    if (wasDown) {
      wasDown = false;
      emitResync();
      void refreshStats();
      refreshTopTags(true);
    }
  };
}

export function stopLive() {
  source?.close();
  source = null;
  wasDown = false;
  pending.clear();
  clearTimeout(flushTimer);
}

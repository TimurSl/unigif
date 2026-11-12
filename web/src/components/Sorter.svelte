<script lang="ts">
  import { onMount } from 'svelte';
  import { api, mediaPath, type Gif } from '../lib/api';
  import { app, emitGif, errorText, loadRecent, noteSaved, refreshStats, setPref, toast } from '../lib/app.svelte';
  import TagSheet from './TagSheet.svelte';

  type Next = { item: Gif | null; remaining: number; total: number };

  let {
    mode,
    startId,
    gif: editGif,
    onclose,
  }: {
    mode: 'queue' | 'edit';
    /** Queue mode: open at this GIF instead of the head of the queue. */
    startId?: string;
    /** Edit mode: the GIF whose hashtags are edited. */
    gif?: Gif;
    onclose: () => void;
  } = $props();

  let current: Gif | null = $state(null);
  let finished = $state(false);
  let ready = $state(false);
  let back: Gif[] = $state([]);
  let forward: Gif[] = [];
  let runTotal = $state(0);
  let handled = $state(0);
  let proceed = $state(app.prefs.proceed_to_next);
  let preload: { after: string; promise: Promise<Next> } | null = null;
  const keepAlive: HTMLVideoElement[] = [];
  const opener = document.activeElement as HTMLElement | null;

  $effect(() => {
    if (proceed !== app.prefs.proceed_to_next) setPref('proceed_to_next', proceed);
  });

  const sort = () => app.prefs.unsorted_sort;

  function fetchNext(after: string | undefined): Promise<Next> {
    if (after && preload?.after === after) return preload.promise;
    const promise = api.unsortedNext(after, sort());
    if (after) preload = { after, promise };
    return promise;
  }

  /** Fetch the item after `g` ahead of time and warm the browser cache with its video. */
  function prefetchAfter(g: Gif) {
    const promise = api.unsortedNext(g.id, sort());
    preload = { after: g.id, promise };
    promise
      .then((n) => {
        const url = n.item?.media?.mp4?.url;
        if (!url) return;
        const v = document.createElement('video');
        v.preload = 'auto';
        v.muted = true;
        v.src = mediaPath(url);
        keepAlive.push(v);
        if (keepAlive.length > 2) keepAlive.shift()!.removeAttribute('src');
      })
      .catch(() => {});
  }

  function show(g: Gif | null) {
    current = g;
    finished = g === null;
    if (g) prefetchAfter(g);
  }

  onMount(() => {
    void loadRecent();
    void start();
    return () => {
      for (const v of keepAlive) v.removeAttribute('src');
      opener?.focus?.();
      void refreshStats();
    };
  });

  async function start() {
    try {
      if (mode === 'edit') {
        current = editGif ?? null;
        ready = true;
        return;
      }
      if (startId) {
        const g = await api.getGif(startId);
        const n = await api.unsortedNext(g.id, sort());
        preload = { after: g.id, promise: Promise.resolve(n) };
        runTotal = n.total;
        handled = Math.max(0, n.total - (n.item ? n.remaining + 1 : 0) - 1);
        show(g);
      } else {
        const n = await api.unsortedNext(undefined, sort());
        runTotal = n.total;
        show(n.item);
      }
    } catch (e) {
      toast(errorText(e), { kind: 'error' });
      onclose();
      return;
    }
    ready = true;
  }

  async function advance() {
    if (!current) return;
    const from = current;
    back.push(from);
    handled++;
    const queued = forward.pop();
    if (queued) {
      show(queued);
      return;
    }
    try {
      const n = await fetchNext(from.id);
      show(n.item);
    } catch (e) {
      toast(errorText(e), { kind: 'error' });
    }
  }

  function prev() {
    const p = back.pop();
    if (!p || !current) return;
    forward.push(current);
    handled = Math.max(0, handled - 1);
    show(p);
  }

  function finish(g: Gif) {
    emitGif('changed', g);
    void refreshStats();
  }

  function save(tags: string[], opposite: boolean) {
    const g = current;
    if (!g) return;
    // "Same as last" and the recent list must not wait for the round trip.
    noteSaved(tags);
    const send = () =>
      api
        .setTags(g.id, tags)
        .then(finish)
        .catch((e) =>
          toast(`Could not save #${tags[0]}…: ${errorText(e)}`, {
            kind: 'error',
            action: { label: 'Retry', run: send },
            ms: 10000,
          }),
        );
    void send();
    if (mode === 'edit') {
      onclose();
      return;
    }
    // Optimistic: the next GIF is shown at once while the request runs.
    if (proceed !== opposite) void advance();
    else onclose();
  }

  function trash() {
    const g = current;
    if (!g) return;
    api
      .trash(g.id)
      .then(() => {
        emitGif('removed', g);
        void refreshStats();
        toast('Moved to trash', {
          action: {
            label: 'Undo',
            run: () =>
              api
                .restore(g.id)
                .then((r) => {
                  emitGif('added', r);
                  void refreshStats();
                })
                .catch((e) => toast(errorText(e), { kind: 'error' })),
          },
          ms: 5000,
        });
      })
      .catch((e) => toast(errorText(e), { kind: 'error' }));
    if (mode === 'edit') onclose();
    else void advance();
  }

  const heading = $derived(
    mode === 'edit' ? 'Edit hashtags' : `Unsorted · ${Math.min(handled + 1, Math.max(runTotal, handled + 1))} of ${Math.max(runTotal, handled + 1)}`,
  );
</script>

<div class="scrim" role="presentation" onmousedown={(e) => e.target === e.currentTarget && onclose()}>
  {#if finished}
    <div class="done" role="dialog" aria-modal="true" aria-label="All sorted" tabindex="-1" onkeydown={(e) => e.key === 'Escape' && onclose()}>
      <p>All sorted.</p>
      <!-- svelte-ignore a11y_autofocus -->
      <button class="btn primary" type="button" autofocus onclick={onclose}>Close</button>
    </div>
  {:else if ready && current}
    {#key current.id}
      <TagSheet
        gif={current}
        {mode}
        {heading}
        bind:proceed
        canGoBack={back.length > 0}
        onsave={save}
        onskip={() => void advance()}
        onprev={prev}
        ontrash={trash}
        {onclose}
      />
    {/key}
  {/if}
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgb(0 0 0 / 0.72);
  }
  .done {
    display: grid;
    gap: 16px;
    justify-items: center;
    padding: 40px 56px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-tile);
    font-size: 20px;
  }
  .done p {
    margin: 0;
  }
  @media (max-width: 720px) {
    .scrim {
      padding: 0;
    }
  }
</style>

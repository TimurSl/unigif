<script lang="ts">
  import { onMount } from 'svelte';
  import { restoreMany } from '../lib/actions';
  import { app, onGifEvent, onResync, refreshTopTags, toast } from '../lib/app.svelte';
  import { Feed } from '../lib/feed.svelte';
  import { Selection } from '../lib/selection.svelte';
  import Grid from './Grid.svelte';
  import SelectionBar from './SelectionBar.svelte';

  const feed = new Feed(() => ({ trash: true, sort: 'new' }));
  const selection = new Selection();
  // Everything in the trash can be picked; there is no other thing a click could mean here.
  selection.active = true;

  $effect(() => selection.keepOnly(feed.items));

  onMount(() => {
    void feed.reset();
    let timer: ReturnType<typeof setTimeout> | undefined;
    // A GIF trashed elsewhere (another tab, the API) only arrives as an id, so the list is fetched again.
    const reload = () => {
      clearTimeout(timer);
      timer = setTimeout(() => void feed.reset(), 250);
    };
    const off = onGifEvent((kind, g) => {
      if (kind === 'added') feed.remove(g.id);
      else if (kind === 'removed') reload();
    });
    const offResync = onResync(reload);
    return () => {
      off();
      offResync();
      clearTimeout(timer);
      feed.destroy();
    };
  });

  function open(g: { id: string }, e?: MouseEvent) {
    if (e?.shiftKey) selection.range(feed.items, g.id);
    else selection.toggle(g.id);
  }

  async function restore() {
    const ids = [...selection.ids];
    const n = await restoreMany(ids);
    for (const id of ids) feed.remove(id);
    selection.clear();
    refreshTopTags(true);
    if (n < ids.length) toast(`${n} restored; ${ids.length - n} could not be.`, { kind: 'error' });
    else toast(`${n} ${n === 1 ? 'GIF' : 'GIFs'} restored`);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && selection.count > 0) selection.clear();
  }
</script>

<svelte:window onkeydown={onKey} />

<section class="list">
  <div class="top">
    <h1>Trash</h1>
    <span class="muted">{app.stats?.trash ?? 0} in trash</span>
  </div>
  <p class="note">Deleted GIFs stay here for a while, then are removed for good. Pick the ones to bring back.</p>

  <Grid
    items={feed.items}
    loading={feed.loading}
    done={feed.done}
    onopen={open}
    onend={() => void feed.more()}
    selecting
    selected={(id) => selection.has(id)}
  >
    {#snippet empty()}
      <p><strong>The trash is empty.</strong></p>
      <p>GIFs you delete wait here before they are removed.</p>
    {/snippet}
  </Grid>

  {#if selection.count > 0}
    <SelectionBar count={selection.count} loaded={feed.items.length} onall={() => selection.all(feed.items)} oncancel={() => selection.clear()}>
      <button class="btn primary" type="button" onclick={restore}>Restore</button>
    </SelectionBar>
  {/if}
</section>

<style>
  .list {
    flex: 1;
    min-width: 0;
    height: 100dvh;
    display: flex;
    flex-direction: column;
  }
  .top {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 12px 16px 0;
  }
  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }
  .muted {
    color: var(--text-muted);
  }
  .note {
    margin: 4px 16px 12px;
    font-size: 13px;
    color: var(--text-faint);
  }
  @media (max-width: 639px) {
    .list {
      height: calc(100dvh - var(--tabbar-h));
    }
    .top {
      padding: calc(8px + env(safe-area-inset-top)) 12px 0;
    }
    .note {
      margin: 4px 12px 8px;
    }
  }
</style>

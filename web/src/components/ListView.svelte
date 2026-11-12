<script lang="ts">
  import { onMount } from 'svelte';
  import { batchChange, restoreMany } from '../lib/actions';
  import { type Gif } from '../lib/api';
  import { app, emitGif, errorText, loadRecent, onGifEvent, onResync, refreshStats, refreshTopTags, setPref, toast } from '../lib/app.svelte';
  import { Feed } from '../lib/feed.svelte';
  import { href, linkClick, navigate, openGif, route } from '../lib/router.svelte';
  import { Selection } from '../lib/selection.svelte';
  import BatchSheet from './BatchSheet.svelte';
  import Grid from './Grid.svelte';
  import SearchBar from './SearchBar.svelte';
  import SelectionBar from './SelectionBar.svelte';
  import Sorter from './Sorter.svelte';
  import TileMenu from './TileMenu.svelte';

  let { view, onupload }: { view: 'library' | 'unsorted'; onupload: () => void } = $props();

  let draft = $state('');
  let searchInput: HTMLInputElement | undefined = $state();
  let sorting: { startId?: string } | null = $state(null);
  /** Phone: the long-press menu, and the tag sheet it can open for one GIF. */
  let menu: Gif | null = $state(null);
  let editing: Gif | null = $state(null);
  const selection = new Selection();
  let batchSheet: 'add' | 'remove' | null = $state(null);

  const chips = $derived(route.q.split(/\s+/).filter(Boolean));
  const query = $derived([...chips, ...(draft.trim() ? [draft.trim()] : [])].join(' '));

  const feed = new Feed(() =>
    view === 'unsorted'
      ? { unsorted: true, sort: app.prefs.unsorted_sort }
      : { q: query || undefined, sort: app.prefs.library_sort },
  );

  const selectedGifs = $derived(feed.items.filter((g) => selection.has(g.id)));

  let lastKey = '';
  let debounce: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const key = `${view}|${view === 'unsorted' ? app.prefs.unsorted_sort : `${app.prefs.library_sort}|${query}`}`;
    if (key === lastKey) return;
    const first = lastKey === '';
    lastKey = key;
    clearTimeout(debounce);
    newResults = false;
    if (first) void feed.reset();
    else debounce = setTimeout(() => void feed.reset(), 120);
  });

  // Ticks follow the list: a GIF that leaves it (sorted, trashed in another tab) is no longer selected.
  $effect(() => selection.keepOnly(feed.items));

  // Changes from the tag sheet, other tabs and the API (live updates) are mirrored in the grid.
  onMount(() => {
    const off = onGifEvent((kind, g) => {
      if (kind === 'removed') return feed.remove(g.id);
      const fits = belongs(g);
      if (fits === false) return feed.remove(g.id);
      if (feed.has(g.id)) return feed.replace(g);
      // New to this list (uploaded, restored, or newly matching the search): put it at its place, or say
      // there is something new when its place is further down than what has been loaded.
      if (fits && !feed.insertSorted(g, view === 'unsorted' ? app.prefs.unsorted_sort : app.prefs.library_sort)) {
        newResults = true;
      }
    });
    const offResync = onResync(reload);
    return () => {
      off();
      offResync();
      clearTimeout(debounce);
      feed.destroy();
    };
  });

  /** Does the GIF carry every hashtag chip and none of the excluded ones? (Library only; free text is left to the server.) */
  function matchesChips(g: Gif): boolean {
    if (view !== 'library') return true;
    return chips.every((c) => (c.startsWith('-') ? !g.tags.includes(c.slice(1)) : g.tags.includes(c)));
  }

  /**
   * Does the GIF belong in the list as it is filtered now? `null` when that cannot be told here: the
   * search box holds a half-typed word, which the server matches as a prefix.
   */
  function belongs(g: Gif): boolean | null {
    if (view === 'unsorted') return g.tags.length === 0;
    if (!matchesChips(g)) return false;
    return draft.trim() === '' ? true : null;
  }

  /** Some GIF now belongs further down than has been loaded. */
  let newResults = $state(false);
  function reload() {
    newResults = false;
    void feed.reset();
  }

  function setChips(next: string[]) {
    navigate(href('library', next.join(' ')), { replace: true });
  }

  function onWindowKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && selection.active && !batchSheet && !sorting && !editing && !menu) {
      selection.stop();
      return;
    }
    if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    if (view !== 'library') return;
    e.preventDefault();
    searchInput?.focus();
  }

  function open(g: Gif, e?: MouseEvent) {
    if (selection.active) {
      if (e?.shiftKey) selection.range(feed.items, g.id);
      else selection.toggle(g.id);
      return;
    }
    // Ctrl or Cmd click starts selecting, as in a file manager.
    if (e && (e.ctrlKey || e.metaKey)) {
      selection.start(g.id);
      return;
    }
    if (view === 'unsorted') sorting = { startId: g.id };
    else openGif(g.id);
  }

  async function applyTags(mode: 'add' | 'remove', tags: string[]) {
    const ids = [...selection.ids];
    batchSheet = null;
    try {
      await batchChange(ids, mode === 'add' ? { add: tags } : { remove: tags });
    } catch (e) {
      toast(errorText(e), { kind: 'error' });
      return;
    }
    for (const g of feed.items.filter((x) => selection.has(x.id))) {
      const next = mode === 'add' ? [...new Set([...g.tags, ...tags])] : g.tags.filter((t) => !tags.includes(t));
      const updated = { ...g, tags: next };
      if ((view === 'unsorted' && next.length > 0) || !matchesChips(updated)) feed.remove(g.id);
      else feed.replace(updated);
    }
    const n = ids.length;
    toast(`${mode === 'add' ? 'Added to' : 'Removed from'} ${n} ${n === 1 ? 'GIF' : 'GIFs'}`);
    selection.stop();
    void refreshStats();
    refreshTopTags(true);
    void loadRecent();
  }

  async function trashSelected() {
    const ids = [...selection.ids];
    try {
      await batchChange(ids, { trash: true });
    } catch (e) {
      toast(errorText(e), { kind: 'error' });
      return;
    }
    for (const id of ids) {
      feed.remove(id);
      emitGif('removed', { id } as Gif);
    }
    selection.stop();
    void refreshStats();
    refreshTopTags(true);
    toast(`${ids.length} ${ids.length === 1 ? 'GIF' : 'GIFs'} moved to trash`, {
      action: { label: 'Undo', run: () => void restoreMany(ids).then(() => refreshTopTags(true)) },
      ms: 6000,
    });
  }

  const total = $derived(app.stats?.gifs ?? 0);
</script>

<svelte:window onkeydown={onWindowKey} />

<section class="list">
  <div class="top">
    {#if view === 'library'}
      <SearchBar {chips} bind:draft bind:input={searchInput} onchips={setChips} />
    {:else}
      <div class="unsorted-head">
        <h1>Unsorted</h1>
        <span class="muted">{app.stats?.unsorted ?? 0} to sort</span>
        <label class="order">
          Order
          <select
            value={app.prefs.unsorted_sort}
            onchange={(e) => {
              app.prefs.unsorted_sort = e.currentTarget.value === 'old' ? 'old' : 'new';
            }}
          >
            <option value="new">Newest first</option>
            <option value="old">Oldest first</option>
          </select>
        </label>
        <button class="btn primary" type="button" disabled={(app.stats?.unsorted ?? 0) === 0} onclick={() => (sorting = {})}>
          Start sorting
        </button>
      </div>
    {/if}
    <button
      class="btn"
      type="button"
      aria-pressed={selection.active}
      disabled={feed.items.length === 0 && !selection.active}
      onclick={() => (selection.active ? selection.stop() : selection.start())}
    >{selection.active ? 'Done' : 'Select'}</button>
    <button class="btn upload" type="button" onclick={onupload}>Upload</button>
  </div>

  {#if view === 'library'}
    <div class="sub">
      <span class="muted">{chips.length === 0 && !draft ? `${total} ${total === 1 ? 'GIF' : 'GIFs'}` : ''}</span>
      <div class="sort" role="group" aria-label="Order" data-testid="library-sort">
        {#each [['new', 'Newest'], ['used', 'Most used'], ['old', 'Oldest']] as [value, label] (value)}
          <button
            type="button"
            aria-pressed={app.prefs.library_sort === value}
            onclick={() => setPref('library_sort', value as 'new' | 'used' | 'old')}>{label}</button
          >
        {/each}
      </div>
    </div>
  {/if}

  {#if view === 'library' && chips.length === 0 && !draft && app.topTags.length}
    <!-- Phone: hashtag browsing lives under the search box (the sidebar is a tab bar there). -->
    <nav class="hashtags" aria-label="Browse hashtags">
      {#each app.topTags.slice(0, 40) as t (t.name)}
        <a class="chip tag" href={href('library', t.name)} onclick={(e) => linkClick(e, href('library', t.name))}>#{t.name}</a>
      {/each}
    </nav>
  {/if}

  {#if newResults}
    <button class="new-results" type="button" onclick={reload} data-testid="new-results">New results · reload</button>
  {/if}

  <Grid
    items={feed.items}
    loading={feed.loading}
    done={feed.done}
    onopen={open}
    onmenu={(g) => (menu = g)}
    onend={() => void feed.more()}
    selecting={selection.active}
    selected={(id) => selection.has(id)}
    onband={(ids) => selection.band(ids)}
    onbandend={() => selection.endBand()}
  >
    {#snippet empty()}
      {#if view === 'unsorted'}
        <p><strong>All sorted.</strong></p>
        <p>New GIFs land here until they get a hashtag.</p>
      {:else if query}
        <p><strong>No GIFs match.</strong></p>
        <p>Try fewer hashtags.</p>
      {:else}
        <p><strong>No GIFs yet.</strong></p>
        <p>Drop files here, paste a link, or press Upload.</p>
      {/if}
    {/snippet}
  </Grid>

  {#if selection.active}
    <SelectionBar count={selection.count} loaded={feed.items.length} onall={() => selection.all(feed.items)} oncancel={() => selection.stop()}>
      <button class="btn" type="button" disabled={selection.count === 0} onclick={() => (batchSheet = 'add')}>Add hashtags</button>
      <button class="btn" type="button" disabled={selection.count === 0} onclick={() => (batchSheet = 'remove')}>Remove hashtags</button>
      <button class="btn danger" type="button" disabled={selection.count === 0} onclick={trashSelected}>Move to trash</button>
    </SelectionBar>
  {/if}
</section>

{#if batchSheet}
  <BatchSheet
    mode={batchSheet}
    gifs={selectedGifs}
    onsubmit={(tags) => batchSheet && void applyTags(batchSheet, tags)}
    onclose={() => (batchSheet = null)}
  />
{/if}

{#if sorting}
  <Sorter mode="queue" startId={sorting.startId} onclose={() => (sorting = null)} />
{/if}
{#if menu}
  <TileMenu gif={menu} onedit={(g) => (editing = g)} onselect={(g) => selection.start(g.id)} onclose={() => (menu = null)} />
{/if}
{#if editing}
  <Sorter mode="edit" gif={editing} onclose={() => (editing = null)} />
{/if}

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
    gap: 12px;
    align-items: center;
    padding: 12px 16px;
  }
  .sub {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 0 16px 8px;
    min-height: 28px;
    font-size: 13px;
  }
  .new-results {
    align-self: center;
    margin: 0 0 8px;
    min-height: 32px;
    padding: 0 14px;
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text);
    font-size: 13px;
  }
  .sort {
    display: inline-flex;
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    overflow: hidden;
  }
  .sort button {
    min-height: 28px;
    padding: 0 12px;
    border: 0;
    background: transparent;
    color: var(--text-muted);
    font-size: 13px;
  }
  .sort button + button {
    border-left: 1px solid var(--border-strong);
  }
  .sort button[aria-pressed='true'] {
    background: var(--accent);
    color: var(--on-accent);
  }
  .muted {
    color: var(--text-muted);
  }
  .unsorted-head {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }
  .order {
    margin-left: auto;
    display: flex;
    gap: 8px;
    align-items: center;
    color: var(--text-muted);
  }
  select {
    min-height: 36px;
    padding: 0 8px;
    background: var(--surface-1);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    color: var(--text);
    font: inherit;
  }
  .hashtags {
    display: none;
  }
  @media (max-width: 639px) {
    .list {
      height: calc(100dvh - var(--tabbar-h));
    }
    .top {
      padding: calc(8px + env(safe-area-inset-top)) 12px 8px;
    }
    .upload {
      display: none; /* the Add tab replaces it */
    }
    .sub {
      padding: 0 12px 4px;
    }
    .hashtags {
      display: flex;
      gap: 8px;
      overflow-x: auto;
      padding: 0 12px 8px;
      scrollbar-width: none;
    }
    .hashtags .chip {
      display: inline-flex;
      align-items: center;
      flex: none;
      min-height: 36px;
      padding: 0 14px;
      border: 1px solid var(--border-strong);
      border-radius: 999px;
      color: var(--text);
      font-size: 13px;
      text-decoration: none;
    }
    .unsorted-head {
      gap: 8px 12px;
    }
    .order {
      margin-left: 0;
    }
  }
</style>

<script lang="ts">
  import { onMount } from 'svelte';
  import { type Gif } from '../lib/api';
  import { app, onGifEvent } from '../lib/app.svelte';
  import { Feed } from '../lib/feed.svelte';
  import { href, linkClick, navigate, openGif, route } from '../lib/router.svelte';
  import Grid from './Grid.svelte';
  import SearchBar from './SearchBar.svelte';
  import Sorter from './Sorter.svelte';
  import TileMenu from './TileMenu.svelte';

  let { view, onupload }: { view: 'library' | 'unsorted'; onupload: () => void } = $props();

  let draft = $state('');
  let searchInput: HTMLInputElement | undefined = $state();
  let sorting: { startId?: string } | null = $state(null);
  /** Phone: the long-press menu, and the tag sheet it can open for one GIF. */
  let menu: Gif | null = $state(null);
  let editing: Gif | null = $state(null);

  const chips = $derived(route.q.split(/\s+/).filter(Boolean));
  const query = $derived([...chips, ...(draft.trim() ? [draft.trim()] : [])].join(' '));

  const feed = new Feed(() =>
    view === 'unsorted'
      ? { unsorted: true, sort: app.prefs.unsorted_sort }
      : { q: query || undefined, sort: 'new' },
  );

  let lastKey = '';
  let debounce: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const key = `${view}|${view === 'unsorted' ? app.prefs.unsorted_sort : query}`;
    if (key === lastKey) return;
    const first = lastKey === '';
    lastKey = key;
    clearTimeout(debounce);
    if (first) void feed.reset();
    else debounce = setTimeout(() => void feed.reset(), 120);
  });

  // A tile that is open in the tag sheet changes under it; mirror that in the grid.
  onMount(() => {
    const off = onGifEvent((kind, g) => {
      if (kind === 'removed') feed.remove(g.id);
      else if (kind === 'changed') {
        if (view === 'unsorted' && g.tags.length > 0) feed.remove(g.id);
        else feed.replace(g);
      } else if (kind === 'added' && matchesTop(g)) feed.prepend(g);
    });
    return () => {
      off();
      clearTimeout(debounce);
      feed.destroy();
    };
  });

  /** New uploads show up at the top of lists they belong in. */
  function matchesTop(g: Gif): boolean {
    if (view === 'unsorted') return g.tags.length === 0 && app.prefs.unsorted_sort === 'new';
    return query === '' || chips.every((c) => c.startsWith('-') || g.tags.includes(c));
  }

  function setChips(next: string[]) {
    navigate(href('library', next.join(' ')), { replace: true });
  }

  function onWindowKey(e: KeyboardEvent) {
    if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    if (view !== 'library') return;
    e.preventDefault();
    searchInput?.focus();
  }

  function open(g: Gif) {
    if (view === 'unsorted') sorting = { startId: g.id };
    else openGif(g.id);
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
    <button class="btn upload" type="button" onclick={onupload}>Upload</button>
  </div>

  {#if view === 'library'}
    <div class="sub">
      <span class="muted">{chips.length === 0 && !draft ? `${total} ${total === 1 ? 'GIF' : 'GIFs'}` : ''}</span>
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

  <Grid items={feed.items} loading={feed.loading} done={feed.done} onopen={open} onmenu={(g) => (menu = g)} onend={() => void feed.more()}>
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
</section>

{#if sorting}
  <Sorter mode="queue" startId={sorting.startId} onclose={() => (sorting = null)} />
{/if}
{#if menu}
  <TileMenu gif={menu} onedit={(g) => (editing = g)} onclose={() => (menu = null)} />
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
    padding: 0 16px 8px;
    min-height: 28px;
    font-size: 13px;
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

<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Gif, type ProviderId, type SearchItem, type SearchProvider } from '../lib/api';
  import { emitGif, errorText, refreshStats, refreshTopTags, toast } from '../lib/app.svelte';
  import { normalizeTag } from '../lib/hashtags';
  import { href, navigate, route } from '../lib/router.svelte';
  import SearchTile from './SearchTile.svelte';
  import SelectionBar from './SelectionBar.svelte';

  type AddState = 'idle' | 'adding' | 'added' | 'have';

  let providers = $state<SearchProvider[] | null>(null);
  let items: SearchItem[] = $state([]);
  let cursor: string | null = null;
  let loading = $state(false);
  let done = $state(true);
  let error = $state('');
  let draft = $state(route.q);
  let tagsDraft = $state('');
  let input: HTMLInputElement | undefined = $state();
  let sentinel: HTMLDivElement | undefined = $state();
  /** What has been added in this visit, by `provider:id`, so it still shows after a new search. */
  let added: Record<string, AddState> = $state({});
  let gen = 0;
  /** Select mode: pick several results and save them together. */
  let selecting = $state(false);
  let picked: string[] = $state([]);
  let lastPicked: string | null = null;
  let saving = $state(false);

  const provider = $derived(providers?.find((p) => p.id === route.provider) ?? providers?.find((p) => p.configured) ?? providers?.[0] ?? null);
  const tags = $derived(
    tagsDraft
      .split(/[\s,]+/)
      .map((t) => normalizeTag(t.replace(/^#+/, '')))
      .filter((t): t is string => !!t),
  );

  onMount(async () => {
    try {
      providers = (await api.searchProviders()).items;
    } catch (e) {
      error = errorText(e);
      providers = [];
    }
  });

  // A new provider or query (from the box, the address bar, or Back) starts a fresh list.
  let lastKey = '';
  $effect(() => {
    const p = provider;
    if (!p) return;
    const key = `${p.id}|${p.configured}|${route.q}`;
    if (key === lastKey) return;
    lastKey = key;
    void reset(p);
  });

  async function reset(p: SearchProvider) {
    gen++;
    picked = [];
    items = [];
    cursor = null;
    error = '';
    done = !p.configured;
    loading = false;
    if (p.configured) await more(p);
  }

  async function more(p: SearchProvider | null = provider) {
    if (!p || !p.configured || loading || done) return;
    const mine = gen;
    loading = true;
    try {
      const page = await api.search({ provider: p.id, q: route.q || undefined, cursor, limit: 30 });
      if (mine !== gen) return;
      const have = new Set(items.map((i) => i.id));
      items.push(...page.items.filter((i) => !have.has(i.id)));
      cursor = page.next_cursor;
      done = page.next_cursor === null || page.items.length === 0;
    } catch (e) {
      if (mine !== gen) return;
      error = errorText(e);
      done = true;
    } finally {
      if (mine === gen) loading = false;
    }
  }

  // Infinite scroll: load the next page when the end of the list comes near.
  $effect(() => {
    if (!sentinel) return;
    const io = new IntersectionObserver(([e]) => e.isIntersecting && void more(), { rootMargin: '800px' });
    io.observe(sentinel);
    return () => io.disconnect();
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    const q = draft.trim();
    navigate(href('search', '') + params(provider?.id, q), { replace: true });
  }

  function params(id: string | undefined, q: string): string {
    const u = new URLSearchParams();
    if (id) u.set('provider', id);
    if (q) u.set('q', q);
    const s = u.toString();
    return s ? `?${s}` : '';
  }

  function pick(id: ProviderId) {
    navigate(`/search${params(id, route.q)}`, { replace: true });
  }

  /** Save one result. Resolves to `added`, `have` or `failed`; `quiet` leaves the messages to the caller. */
  async function add(item: SearchItem, quiet = false): Promise<'added' | 'have' | 'failed'> {
    const p = provider;
    if (!p) return 'failed';
    const k = `${p.id}:${item.id}`;
    added[k] = 'adding';
    try {
      const gif: Gif = await api.addFromSearch(p.id, item, tags);
      added[k] = gif.duplicate ? 'have' : 'added';
      if (gif.duplicate && !quiet) toast(tags.length ? 'Already in your library; hashtags were merged in.' : 'Already in your library.');
      emitGif('added', gif);
      if (!quiet) {
        void refreshStats();
        if (tags.length) refreshTopTags(true);
      }
      return added[k] as 'added' | 'have';
    } catch (e) {
      added[k] = 'idle';
      if (!quiet) toast(errorText(e), { kind: 'error' });
      return 'failed';
    }
  }

  const status = (item: SearchItem) => added[`${provider?.id}:${item.id}`] ?? 'idle';
  const pickable = $derived(items.filter((i) => status(i) === 'idle'));

  function toggle(item: SearchItem, e: MouseEvent) {
    // Shift-click ticks everything between the last pick and this one, as in the library.
    if (e.shiftKey && lastPicked) {
      const ids = pickable.map((i) => i.id);
      const [a, b] = [ids.indexOf(lastPicked), ids.indexOf(item.id)].sort((x, y) => x - y);
      if (a >= 0) {
        picked = [...new Set([...picked, ...ids.slice(a, b + 1)])];
        lastPicked = item.id;
        return;
      }
    }
    picked = picked.includes(item.id) ? picked.filter((id) => id !== item.id) : [...picked, item.id];
    lastPicked = item.id;
  }

  function stopSelecting() {
    selecting = false;
    picked = [];
    lastPicked = null;
  }

  /** Save every ticked result, six at a time, with the hashtags in the box. */
  async function saveSelected() {
    const queue = items.filter((i) => picked.includes(i.id) && status(i) === 'idle');
    if (!queue.length) return;
    saving = true;
    const total = queue.length;
    const counts = { added: 0, have: 0, failed: 0 };
    const worker = async () => {
      for (let item = queue.shift(); item; item = queue.shift()) counts[await add(item, true)]++;
    };
    await Promise.all(Array.from({ length: Math.min(6, total) }, worker));
    saving = false;
    void refreshStats();
    if (tags.length) refreshTopTags(true);
    const parts = [`${counts.added} saved`];
    if (counts.have) parts.push(`${counts.have} already in your library`);
    if (counts.failed) parts.push(`${counts.failed} failed`);
    toast(parts.join(', '), { kind: counts.failed ? 'error' : 'info' });
    // Keep the failed ones ticked so they can be retried.
    picked = picked.filter((id) => items.some((i) => i.id === id && status(i) === 'idle'));
    if (picked.length === 0) stopSelecting();
  }

  function onWindowKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && selecting && !saving) {
      stopSelecting();
      return;
    }
    if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    e.preventDefault();
    input?.focus();
  }
</script>

<svelte:window onkeydown={onWindowKey} />

<section class="page">
  <div class="top">
    <form class="bar" role="search" onsubmit={submit}>
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7" /><path d="m20 20-3.5-3.5" /></svg>
      <input
        bind:this={input}
        bind:value={draft}
        type="search"
        aria-label="Search {provider?.name ?? 'GIFs'}"
        placeholder={provider ? `Search ${provider.name}…` : 'Search…'}
        autocomplete="off"
        spellcheck="false"
      />
      <kbd aria-hidden="true">/</kbd>
    </form>
    {#if providers && providers.length}
      <div class="providers" role="tablist" aria-label="Search provider">
        {#each providers as p (p.id)}
          <button
            class="seg"
            class:current={provider?.id === p.id}
            type="button"
            role="tab"
            aria-selected={provider?.id === p.id}
            onclick={() => pick(p.id)}>{p.name}</button
          >
        {/each}
      </div>
    {/if}
    {#if provider?.configured}
      <button
        class="btn"
        type="button"
        aria-pressed={selecting}
        disabled={!selecting && pickable.length === 0}
        onclick={() => (selecting ? stopSelecting() : (selecting = true))}>{selecting ? 'Done' : 'Select'}</button
      >
    {/if}
  </div>

  <div class="sub">
    <label class="tags">
      <span>Add with hashtags</span>
      <input
        class="tag"
        bind:value={tagsDraft}
        aria-label="Hashtags for added GIFs"
        placeholder="optional, e.g. cat funny"
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
      />
    </label>
    <span class="muted hint">{tags.length ? `Added GIFs get ${tags.map((t) => '#' + t).join(' ')}.` : 'Without hashtags, added GIFs wait in Unsorted.'}</span>
  </div>

  <div class="results">
    {#if providers === null}
      <p class="status">Loading…</p>
    {:else if provider && !provider.configured}
      <div class="empty">
        <p><strong>{provider.name} needs an API key.</strong></p>
        <p>Add one in Settings to search {provider.name} and save GIFs to your library.</p>
        <p><a class="btn primary" href="/settings" onclick={(e) => { e.preventDefault(); navigate('/settings'); }}>Open Settings</a></p>
      </div>
    {:else if error && items.length === 0}
      <div class="empty" role="alert">
        <p><strong>Search failed.</strong></p>
        <p>{error}</p>
        <p><button class="btn" type="button" onclick={() => provider && void reset(provider)}>Try again</button></p>
      </div>
    {:else}
      {#if !route.q && items.length}<p class="muted label">Trending on {provider?.name}</p>{/if}
      <div class="grid">
        {#each items as item (item.id)}
          <SearchTile
            {item}
            status={status(item)}
            onadd={(i) => void add(i)}
            {selecting}
            selected={picked.includes(item.id)}
            ontoggle={toggle}
          />
        {/each}
      </div>
      {#if loading}<p class="status">Loading…</p>{/if}
      {#if !loading && done && items.length === 0}
        <div class="empty"><p><strong>No results.</strong></p><p>Try other words.</p></div>
      {/if}
      {#if error && items.length}<p class="status" role="alert">{error}</p>{/if}
      <div bind:this={sentinel} class="sentinel"></div>
    {/if}
  </div>

  {#if selecting}
    <SelectionBar
      count={picked.length}
      loaded={pickable.length}
      onall={() => (picked = pickable.map((i) => i.id))}
      oncancel={stopSelecting}
    >
      <button class="btn primary" type="button" disabled={picked.length === 0 || saving} onclick={saveSelected}>
        {saving ? 'Saving…' : `Save ${picked.length || ''}`.trim()}
      </button>
    </SelectionBar>
  {/if}
</section>

<style>
  .page {
    flex: 1;
    min-width: 0;
    height: 100dvh;
    display: flex;
    flex-direction: column;
  }
  .top {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    align-items: center;
    padding: 12px 16px 8px;
  }
  .bar {
    flex: 1;
    min-width: 220px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px;
    min-height: 40px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    color: var(--text-muted);
  }
  .bar:focus-within {
    border-color: var(--border-strong);
  }
  .bar input {
    flex: 1;
    min-width: 0;
    min-height: 30px;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: 16px;
  }
  .bar input::-webkit-search-cancel-button {
    display: none;
  }
  .providers {
    display: flex;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    overflow: hidden;
  }
  .seg {
    min-height: 38px;
    padding: 0 14px;
    border: 0;
    background: transparent;
    color: var(--text-muted);
  }
  .seg.current {
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 500;
  }
  .sub {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 16px;
    padding: 0 16px 8px;
    font-size: 13px;
  }
  .tags {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-muted);
  }
  .tags input {
    min-height: 32px;
    width: 220px;
    max-width: 100%;
    padding: 0 8px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    color: var(--text);
    font-size: 16px;
  }
  .muted {
    color: var(--text-muted);
  }
  .results {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 16px 16px;
  }
  .grid {
    columns: 220px;
    column-gap: 12px;
  }
  .label {
    margin: 0 0 8px;
    font-size: 12px;
  }
  .status {
    text-align: center;
    color: var(--text-faint);
    margin: 12px 0 0;
  }
  .sentinel {
    height: 1px;
  }
  .empty {
    display: grid;
    place-content: center;
    min-height: 50%;
    text-align: center;
    color: var(--text-muted);
    gap: 8px;
  }
  .empty p {
    margin: 0;
  }
  .empty .btn {
    text-decoration: none;
    color: inherit;
  }
  .empty .btn.primary {
    color: var(--on-accent);
  }
  @media (max-width: 639px) {
    .page {
      height: calc(100dvh - var(--tabbar-h));
    }
    .top {
      padding: calc(8px + env(safe-area-inset-top)) 12px 8px;
    }
    .sub,
    .results {
      padding-left: 12px;
      padding-right: 12px;
    }
    .grid {
      columns: 2;
    }
    kbd,
    .hint {
      display: none;
    }
    .tags {
      flex: 1;
    }
    .tags input {
      flex: 1;
      width: auto;
    }
  }
</style>

<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Gif, type ProviderId, type SearchItem, type SearchProvider } from '../lib/api';
  import { emitGif, errorText, refreshStats, refreshTopTags, toast } from '../lib/app.svelte';
  import { normalizeTag } from '../lib/hashtags';
  import { href, navigate, route } from '../lib/router.svelte';
  import SearchTile from './SearchTile.svelte';

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

  async function add(item: SearchItem) {
    const p = provider;
    if (!p) return;
    const k = `${p.id}:${item.id}`;
    added[k] = 'adding';
    try {
      const gif: Gif = await api.addFromSearch(p.id, item, tags);
      added[k] = gif.duplicate ? 'have' : 'added';
      if (gif.duplicate) toast(tags.length ? 'Already in your library; hashtags were merged in.' : 'Already in your library.');
      emitGif('added', gif);
      void refreshStats();
      if (tags.length) refreshTopTags(true);
    } catch (e) {
      added[k] = 'idle';
      toast(errorText(e), { kind: 'error' });
    }
  }

  function onWindowKey(e: KeyboardEvent) {
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
          <SearchTile {item} status={added[`${provider?.id}:${item.id}`] ?? 'idle'} onadd={add} />
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

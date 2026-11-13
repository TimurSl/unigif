<script lang="ts">
  import { onMount } from 'svelte';
  import { api, ApiError, type Tag } from '../lib/api';
  import { app, errorText, refreshStats, refreshTopTags, toast, onResync } from '../lib/app.svelte';
  import { normalizeTag } from '../lib/hashtags';
  import { href, linkClick } from '../lib/router.svelte';

  type Sort = 'alpha' | 'popular' | 'recent';
  type Action = { name: string; kind: 'rename' | 'merge' | 'delete'; value: string; error: string; busy: boolean; conflict: boolean };

  let sort: Sort = $state('popular');
  let filter = $state('');
  let tags: Tag[] = $state([]);
  let cursor: string | null = $state(null);
  let loading = $state(false);
  let loaded = $state(false);
  let action = $state<Action | null>(null);
  let gen = 0;

  // Renaming, merging and deleting reach every GIF, so they need the tag scope and a key without hidden hashtags.
  const canEdit = $derived(!!app.me && app.me.scopes.includes('tag') && app.me.deny_tags.length === 0);

  async function load(reset: boolean) {
    const mine = reset ? ++gen : gen;
    if (reset) {
      cursor = null;
      tags = [];
      loaded = false;
    }
    loading = true;
    try {
      const q = normalizeTag(filter) ?? undefined;
      const page = await api.listTagsPage({ q, sort, limit: 100, cursor: reset ? null : cursor });
      if (mine !== gen) return;
      tags = reset ? page.items : [...tags, ...page.items];
      cursor = page.next_cursor;
    } catch (e) {
      if (mine === gen) toast(errorText(e), { kind: 'error' });
    } finally {
      if (mine === gen) {
        loading = false;
        loaded = true;
      }
    }
  }

  let timer: ReturnType<typeof setTimeout> | undefined;
  let first = true;
  $effect(() => {
    void sort;
    void filter;
    clearTimeout(timer);
    if (first) {
      first = false;
      void load(true);
    } else timer = setTimeout(() => void load(true), 150);
  });

  onMount(() => {
    const off = onResync(() => void load(true));
    return () => {
      off();
      clearTimeout(timer);
    };
  });

  function begin(t: Tag, kind: Action['kind']) {
    action = { name: t.name, kind, value: kind === 'rename' ? t.name : '', error: '', busy: false, conflict: false };
  }

  /** Shown after a change: the list, the sidebar and the numbers all moved. */
  function changed() {
    void load(true);
    refreshTopTags();
    void refreshStats();
  }

  async function run(fn: () => Promise<unknown>, done: string) {
    if (!action) return;
    action.busy = true;
    action.error = '';
    action.conflict = false;
    try {
      await fn();
      action = null;
      toast(done);
      changed();
    } catch (e) {
      if (action) {
        action.busy = false;
        action.error = errorText(e);
        action.conflict = e instanceof ApiError && e.status === 409;
      }
    }
  }

  function submit() {
    if (!action) return;
    const a = action;
    const target = normalizeTag(a.value);
    if (a.kind === 'delete') return run(() => api.deleteTag(a.name), `#${a.name} removed from every GIF`);
    if (!target) {
      a.error = 'Type a hashtag.';
      return;
    }
    if (target === a.name) {
      action = null;
      return;
    }
    if (a.kind === 'rename') {
      return run(() => api.renameTag(a.name, target), `#${a.name} is now #${target}`);
    }
    return run(() => api.mergeTag(a.name, target), `#${a.name} merged into #${target}`);
  }

  /** A rename onto an existing hashtag is a merge in disguise; offer it instead of just failing. */
  const clash = $derived(action?.kind === 'rename' && action.conflict);
  function mergeInstead() {
    if (!action) return;
    action = { ...action, kind: 'merge', error: '', conflict: false };
  }

  const countOf = (name: string) => tags.find((t) => t.name === name)?.gif_count ?? 0;
</script>

<section class="page">
  <h1>Hashtags</h1>
  <p class="lead">Rename a hashtag, merge duplicates into one, or remove one from every GIF. Changes apply to the whole library.</p>

  {#if !canEdit}
    <p class="warn" role="note">This sign-in cannot change hashtags library-wide.</p>
  {/if}

  <div class="tools">
    <input
      type="search"
      bind:value={filter}
      placeholder="Filter hashtags"
      aria-label="Filter hashtags"
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
    />
    <label class="order">
      Order
      <select bind:value={sort}>
        <option value="popular">Most used</option>
        <option value="alpha">A to Z</option>
        <option value="recent">Recently used</option>
      </select>
    </label>
  </div>

  {#if loaded && tags.length === 0}
    <p class="empty">{filter ? 'No hashtag starts with that.' : 'No hashtags yet. They appear as you sort GIFs.'}</p>
  {/if}

  <ul class="rows" data-testid="hashtag-list">
    {#each tags as t (t.name)}
      <li>
        <div class="row">
          <a class="name tag" href={href('library', t.name)} onclick={(e) => linkClick(e, href('library', t.name))}>#{t.name}</a>
          <span class="count" title="GIFs with this hashtag">{t.gif_count}</span>
          {#if canEdit}
            <span class="acts">
              <button class="btn" type="button" aria-label="Rename #{t.name}" onclick={() => begin(t, 'rename')}>Rename</button>
              <button class="btn" type="button" aria-label="Merge #{t.name} into another hashtag" onclick={() => begin(t, 'merge')}>Merge…</button>
              <button class="btn danger" type="button" aria-label="Delete #{t.name}" onclick={() => begin(t, 'delete')}>Delete</button>
            </span>
          {/if}
        </div>
        {#if action && action.name === t.name}
          <form
            class="edit"
            onsubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
          >
            {#if action.kind === 'delete'}
              <p>
                Remove <strong class="tag">#{t.name}</strong> from {countOf(t.name)} {countOf(t.name) === 1 ? 'GIF' : 'GIFs'}? The GIFs stay; only this hashtag goes.
              </p>
              <div class="buttons">
                <button class="btn danger" type="submit" disabled={action.busy}>Remove hashtag</button>
                <button class="btn" type="button" onclick={() => (action = null)}>Cancel</button>
              </div>
            {:else}
              <label>
                {action.kind === 'rename' ? 'New name' : 'Merge into'}
                <input
                  class="tag"
                  list="known-hashtags"
                  bind:value={action.value}
                  aria-label={action.kind === 'rename' ? `New name for #${t.name}` : `Hashtag to merge #${t.name} into`}
                  autocomplete="off"
                  autocapitalize="off"
                  spellcheck="false"
                  placeholder={action.kind === 'rename' ? t.name : 'other_hashtag'}
                />
              </label>
              {#if action.kind === 'merge'}
                <datalist id="known-hashtags">
                  {#each app.topTags.filter((x) => x.name !== t.name) as o (o.name)}<option value={o.name}></option>{/each}
                </datalist>
                <p class="hint">Every GIF with <span class="tag">#{t.name}</span> gets the other hashtag, and <span class="tag">#{t.name}</span> goes away.</p>
              {/if}
              {#if action.error}<p class="error" role="alert">{action.error}</p>{/if}
              <div class="buttons">
                <button class="btn primary" type="submit" disabled={action.busy || !normalizeTag(action.value)}>
                  {action.kind === 'rename' ? 'Rename' : 'Merge'}
                </button>
                {#if clash}<button class="btn" type="button" onclick={mergeInstead}>Merge into it instead</button>{/if}
                <button class="btn" type="button" onclick={() => (action = null)}>Cancel</button>
              </div>
            {/if}
          </form>
        {/if}
      </li>
    {/each}
  </ul>

  {#if cursor}
    <button class="btn more" type="button" disabled={loading} onclick={() => void load(false)}>{loading ? 'Loading…' : 'Show more'}</button>
  {/if}
</section>

<style>
  .page {
    flex: 1;
    min-width: 0;
    height: 100dvh;
    overflow-y: auto;
    padding: 16px;
    align-content: start;
    /* Rows take their content's height; without this a fixed-height grid squeezes the clipped lists. */
    grid-auto-rows: max-content;
    display: grid;
    /* One column that never grows past the screen: an implicit auto column widens to its widest child. */
    grid-template-columns: minmax(0, 1fr);
    gap: 12px;
  }
  .page > * {
    max-width: 720px;
  }
  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }
  .lead,
  .empty {
    margin: 0;
    color: var(--text-muted);
  }
  .warn {
    margin: 0;
    padding: 8px 12px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    color: var(--text-muted);
  }
  .tools {
    display: flex;
    gap: 12px;
    align-items: center;
  }
  .tools input {
    flex: 1;
    min-width: 0;
  }
  input,
  select {
    min-height: 36px;
    padding: 0 10px;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    color: var(--text);
    font-size: 16px;
  }
  .order {
    display: flex;
    gap: 8px;
    align-items: center;
    color: var(--text-muted);
  }
  select {
    font-size: 14px;
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-tile);
    overflow: hidden;
  }
  li + li {
    border-top: 1px solid var(--border);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 48px;
    padding: 6px 12px;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    text-decoration: none;
  }
  .name:hover {
    text-decoration: underline;
  }
  .count {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .acts {
    display: flex;
    gap: 6px;
  }
  .acts .btn {
    min-height: 32px;
    padding: 0 10px;
    font-size: 13px;
  }
  .edit {
    display: grid;
    gap: 8px;
    padding: 8px 12px 12px;
    background: var(--surface-1);
  }
  .edit label {
    display: grid;
    gap: 4px;
    color: var(--text-muted);
  }
  .edit p {
    margin: 0;
  }
  .hint {
    font-size: 12px;
    color: var(--text-faint);
  }
  .error {
    color: var(--danger);
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .more {
    justify-self: start;
  }
  @media (max-width: 639px) {
    .page {
      height: calc(100dvh - var(--tabbar-h));
      padding: calc(12px + env(safe-area-inset-top)) 12px 12px;
    }
    .row {
      flex-wrap: wrap;
    }
    .acts {
      width: 100%;
    }
    .acts .btn {
      flex: 1;
      min-height: 44px;
    }
    .tools {
      flex-wrap: wrap;
    }
  }
</style>

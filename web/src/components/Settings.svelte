<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type ProviderId, type SearchProvider } from '../lib/api';
  import { app, errorText, logout, toast } from '../lib/app.svelte';
  import { install, isIos, pwa } from '../lib/pwa.svelte';

  const size = (n: number) =>
    n >= 1073741824 ? `${(n / 1073741824).toFixed(1)} GB` : n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(0, Math.round(n / 1024))} kB`;

  let providers: SearchProvider[] = $state([]);
  let keys: Record<string, string> = $state({});
  let busy = $state('');

  onMount(async () => {
    try {
      providers = (await api.searchProviders()).items;
    } catch {
      /* the card just stays empty */
    }
  });

  async function saveKey(id: ProviderId, key: string | null) {
    busy = id;
    try {
      const updated = await api.setProviderKey(id, key);
      providers = providers.map((p) => (p.id === id ? updated : p));
      keys[id] = '';
      toast(key ? `${updated.name} key saved.` : `${updated.name} key removed.`);
    } catch (e) {
      toast(errorText(e), { kind: 'error' });
    } finally {
      busy = '';
    }
  }
</script>

<section class="page">
  <h1>Settings</h1>

  <div class="card">
    <h2>Account</h2>
    <p>Signed in as <strong>{app.me?.name}</strong></p>
    <button class="btn danger" type="button" onclick={logout}>Sign out</button>
  </div>

  {#if providers.length}
    <div class="card" data-testid="search-settings">
      <h2>GIF search</h2>
      <p>Search KLIPY and GIPHY from the Search tab and save results to your library. Each needs its own free API key; keys stay on the server.</p>
      {#each providers as p (p.id)}
        <form
          class="provider"
          onsubmit={(e) => {
            e.preventDefault();
            if (keys[p.id]?.trim()) void saveKey(p.id, keys[p.id].trim());
          }}
        >
          <div class="provider-head">
            <strong>{p.name}</strong>
            <span class="state" class:on={p.configured}>
              {p.configured ? (p.key_source === 'config' ? 'Key set in the server configuration' : 'Key saved') : 'No key'}
            </span>
            <a href={p.keys_url} target="_blank" rel="noreferrer noopener">Get a key</a>
          </div>
          <div class="row">
            <input
              type="password"
              bind:value={keys[p.id]}
              placeholder={p.configured ? 'Enter a new key to replace it' : 'Paste your API key'}
              aria-label="{p.name} API key"
              autocomplete="off"
              autocapitalize="off"
              spellcheck="false"
            />
            <button class="btn primary" type="submit" disabled={busy === p.id || !keys[p.id]?.trim()}>Save</button>
            {#if p.key_source === 'settings'}
              <button class="btn" type="button" disabled={busy === p.id} onclick={() => saveKey(p.id, null)}>Remove</button>
            {/if}
          </div>
        </form>
      {/each}
      <p class="hint">Tenor is not available: Google shut down its API for developers on 30 June 2026.</p>
    </div>
  {/if}

  {#if app.stats}
    <div class="card">
      <h2>Storage</h2>
      <dl>
        <dt>GIFs</dt><dd>{app.stats.gifs}</dd>
        <dt>Stored</dt><dd>{size(app.stats.bytes_stored)}</dd>
        <dt>Saved by conversion</dt><dd>{size(app.stats.bytes_saved)}</dd>
        <dt>Converted copies</dt>
        <dd>{app.stats.cache_files} files, {size(app.stats.cache_bytes)} of {size(app.stats.cache_limit)}</dd>
      </dl>
      <p class="hint">GIF, WebM and WebP are made the first time they are asked for. The oldest are removed when the limit is reached; they are rebuilt on demand.</p>
    </div>
  {/if}

  {#if !pwa.standalone}
    <div class="card">
      <h2>App</h2>
      {#if pwa.installPrompt}
        <p>Install unigif to open it like an app and share GIFs into it from other apps.</p>
        <button class="btn primary" type="button" onclick={install}>Install app</button>
      {:else if isIos}
        <p>On iPhone and iPad, tap Share in Safari, then <strong>Add to Home Screen</strong>. Safari cannot receive shares; use Add, or an iOS Shortcut that posts to the quick-add endpoint.</p>
      {:else}
        <p>Use your browser's menu to install unigif as an app. Once installed, Android lets you share GIFs straight into it.</p>
      {/if}
    </div>
  {/if}
</section>

<style>
  .page {
    flex: 1;
    min-width: 0;
    height: 100dvh;
    overflow-y: auto;
    padding: 16px;
    display: grid;
    align-content: start;
    gap: 16px;
  }
  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }
  h2 {
    margin: 0 0 8px;
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .card {
    max-width: 560px;
    padding: 16px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-tile);
  }
  p {
    margin: 0 0 12px;
    color: var(--text-muted);
  }
  p strong {
    color: var(--text);
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 16px;
    margin: 0;
  }
  .hint {
    margin: 12px 0 0;
    font-size: 12px;
    color: var(--text-faint);
  }
  .provider {
    margin: 0 0 16px;
  }
  .provider-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 12px;
    margin-bottom: 6px;
  }
  .provider-head a {
    margin-left: auto;
    color: var(--text-muted);
  }
  .state {
    font-size: 12px;
    color: var(--text-faint);
  }
  .state.on {
    color: var(--text-muted);
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .row input {
    flex: 1;
    min-width: 0;
    min-height: 36px;
    padding: 0 10px;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    color: var(--text);
    font-size: 16px;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
  }
  @media (max-width: 639px) {
    .page {
      height: calc(100dvh - var(--tabbar-h));
    }
    .btn {
      min-height: 44px;
    }
  }
</style>

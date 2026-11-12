<script lang="ts">
  import { onMount } from 'svelte';
  import { api, ApiError, type ProviderId, type SearchProvider, type Session } from '../lib/api';
  import { app, errorText, logout, refreshStats, toast } from '../lib/app.svelte';
  import { ago, describeDevice, formatBytes as size } from '../lib/format';
  import { install, isIos, pwa } from '../lib/pwa.svelte';
  import { linkClick } from '../lib/router.svelte';

  let providers: SearchProvider[] = $state([]);
  let keys: Record<string, string> = $state({});
  let busy = $state('');

  const isOwner = $derived(app.me?.kind === 'user');
  const isAdmin = $derived(app.me?.scopes.includes('admin') ?? false);

  let sessions: Session[] = $state([]);
  let sessionsFailed = $state(false);

  let pwCurrent = $state('');
  let pwNew = $state('');
  let pwRepeat = $state('');
  let pwError = $state('');
  const pwMismatch = $derived(pwRepeat.length > 0 && pwNew !== pwRepeat);

  async function loadSessions() {
    try {
      sessions = (await api.listSessions()).items;
      sessionsFailed = false;
    } catch {
      sessionsFailed = true;
    }
  }

  onMount(async () => {
    if (isOwner) void loadSessions();
    try {
      providers = (await api.searchProviders()).items;
    } catch {
      /* the card just stays empty */
    }
  });

  async function changePassword(e: SubmitEvent) {
    e.preventDefault();
    pwError = '';
    if (pwNew !== pwRepeat) {
      pwError = 'The new passwords do not match.';
      return;
    }
    busy = 'password';
    try {
      await api.changePassword(pwCurrent, pwNew);
      pwCurrent = pwNew = pwRepeat = '';
      toast('Password changed. Every other device was signed out.');
      void loadSessions();
    } catch (err) {
      pwError = err instanceof ApiError && err.code === 'invalid_credentials' ? 'The current password is wrong.' : errorText(err);
    } finally {
      busy = '';
    }
  }

  async function revoke(s: Session) {
    busy = `session:${s.id}`;
    try {
      await api.revokeSession(s.id);
      sessions = sessions.filter((x) => x.id !== s.id);
      toast(`${describeDevice(s.user_agent)} signed out.`);
    } catch (err) {
      toast(errorText(err), { kind: 'error' });
    } finally {
      busy = '';
    }
  }

  async function clearCache() {
    busy = 'cache';
    try {
      const r = await api.clearCache();
      toast(r.files ? `Cleared ${r.files} converted ${r.files === 1 ? 'file' : 'files'}, ${size(r.bytes)} freed.` : 'The cache was already empty.');
      await refreshStats();
    } catch (err) {
      toast(errorText(err), { kind: 'error' });
    } finally {
      busy = '';
    }
  }

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

  <div class="card" data-testid="account-card">
    <h2>Account</h2>
    <p>Signed in as <strong>{app.me?.name}</strong></p>
    <button class="btn danger" type="button" onclick={logout}>Sign out</button>

    {#if isOwner}
      <form class="password" onsubmit={changePassword} data-testid="password-form">
        <h3>Change password</h3>
        <!-- A hidden username lets password managers file the new password under the right account. -->
        <input type="text" name="username" autocomplete="username" value={app.me?.name ?? ''} hidden readonly />
        <label>
          <span>Current password</span>
          <input type="password" bind:value={pwCurrent} autocomplete="current-password" required />
        </label>
        <label>
          <span>New password</span>
          <input type="password" bind:value={pwNew} autocomplete="new-password" minlength="8" required />
        </label>
        <label>
          <span>Repeat new password</span>
          <input type="password" bind:value={pwRepeat} autocomplete="new-password" required aria-invalid={pwMismatch} />
        </label>
        {#if pwError || pwMismatch}
          <p class="error" role="alert">{pwError || 'The new passwords do not match.'}</p>
        {/if}
        <p class="hint">Every other device is signed out; this one stays signed in.</p>
        <button class="btn primary" type="submit" disabled={busy === 'password' || !pwCurrent || !pwNew || pwMismatch}>Change password</button>
      </form>

      <div class="sessions" data-testid="sessions">
        <h3>Signed-in devices</h3>
        {#if sessionsFailed}
          <p class="hint">Could not load the list.</p>
        {:else}
          <ul>
            {#each sessions as s (s.id)}
              <li>
                <div class="who">
                  <strong>{describeDevice(s.user_agent)}</strong>
                  {#if s.current}<span class="this">This device</span>{/if}
                  <span class="meta">{s.ip ?? 'unknown address'} · last seen {ago(s.last_seen_at)}</span>
                </div>
                {#if !s.current}
                  <button class="btn" type="button" disabled={busy === `session:${s.id}`} onclick={() => revoke(s)}>Revoke</button>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  </div>

  <div class="card" data-testid="manage-card">
    <h2>Manage</h2>
    <ul class="links">
      <li><a href="/tags" onclick={(e) => linkClick(e, '/tags')}>Hashtags</a><span>Rename, merge or remove</span></li>
      <li><a href="/trash" onclick={(e) => linkClick(e, '/trash')}>Trash</a><span>{app.stats?.trash ?? 0} deleted GIFs</span></li>
      <li><a href="/keys" onclick={(e) => linkClick(e, '/keys')}>API keys</a><span>Let other apps and people in</span></li>
    </ul>
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
      {#if isAdmin}
        <button class="btn spaced" type="button" disabled={busy === 'cache' || app.stats.cache_files === 0} onclick={clearCache}>Clear cache</button>
      {/if}
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
  h3 {
    margin: 0 0 8px;
    font-size: 14px;
    font-weight: 500;
  }
  .password,
  .sessions {
    margin-top: 20px;
    padding-top: 16px;
    border-top: 1px solid var(--border);
  }
  .password {
    display: grid;
    gap: 10px;
  }
  .password h3,
  .password p {
    margin: 0;
  }
  .password label {
    display: grid;
    gap: 4px;
    font-size: 13px;
    color: var(--text-muted);
  }
  .password input {
    min-height: 36px;
    padding: 0 10px;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    color: var(--text);
    font-size: 16px;
  }
  .password .btn {
    justify-self: start;
  }
  .error {
    color: var(--danger);
    font-size: 13px;
  }
  .sessions ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .sessions li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 0;
    border-top: 1px solid var(--border);
  }
  .sessions li:first-child {
    border-top: 0;
  }
  .who {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 2px 8px;
    min-width: 0;
  }
  .this {
    font-size: 12px;
    padding: 1px 8px;
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    color: var(--text-muted);
  }
  .meta {
    flex-basis: 100%;
    font-size: 12px;
    color: var(--text-faint);
    overflow-wrap: anywhere;
  }
  .spaced {
    margin-top: 12px;
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
  .links {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
  }
  .links li {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    align-items: baseline;
    padding: 10px 0;
    border-top: 1px solid var(--border);
  }
  .links li:first-child {
    border-top: 0;
    padding-top: 0;
  }
  .links a {
    color: var(--text);
    font-weight: 500;
  }
  .links span {
    color: var(--text-faint);
    font-size: 13px;
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

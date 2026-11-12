<script lang="ts">
  import { onMount } from 'svelte';
  import { copyText } from '../lib/actions';
  import { api, ApiError, type ApiKey, type KeyPreset } from '../lib/api';
  import { app, errorText, toast } from '../lib/app.svelte';
  import { parseTags } from '../lib/hashtags';

  const presets: { id: KeyPreset; label: string; help: string }[] = [
    { id: 'read_only', label: 'Read only', help: 'Browse and search. For a friend or a public picker.' },
    { id: 'picker', label: 'Picker', help: 'Read, and add GIFs. For a chat client or a social network.' },
    { id: 'uploader', label: 'Upload only', help: 'Add GIFs and nothing else. For a bot or a share hook.' },
    { id: 'full', label: 'Full', help: 'Read, add, tag and delete. Everything except managing keys.' },
  ];

  const presetOf = (scopes: string[]): string => {
    const has = (s: string) => scopes.includes(s);
    if (has('admin')) return 'Admin';
    if (has('tag') && has('delete')) return 'Full';
    if (has('read') && has('upload')) return 'Picker';
    if (has('read') && scopes.length === 1) return 'Read only';
    if (has('upload') && scopes.length === 1) return 'Upload only';
    return scopes.join(', ');
  };
  const presetId = (scopes: string[]): KeyPreset | null =>
    ({ 'Read only': 'read_only', Picker: 'picker', 'Upload only': 'uploader', Full: 'full' } as Record<string, KeyPreset>)[presetOf(scopes)] ?? null;

  let keys: ApiKey[] = $state([]);
  let loaded = $state(false);
  let showRevoked = $state(false);
  let denied = $state(false);

  // The create form.
  let name = $state('');
  let preset: KeyPreset = $state('read_only');
  let hidden = $state('');
  let expiry = $state('0');
  let rate = $state('');
  let creating = $state(false);
  let formError = $state('');
  /** The secret of the key just made. It is shown here once and the server never returns it again. */
  let secret: { name: string; key: string } | null = $state(null);

  // Editing one key.
  let editing: { id: string; name: string; preset: KeyPreset; hidden: string; error: string; busy: boolean } | null = $state(null);
  let revoking: string | null = $state(null);

  async function load() {
    try {
      keys = (await api.listKeys(showRevoked)).items;
      denied = false;
    } catch (e) {
      if (e instanceof ApiError && e.status === 403) denied = true;
      else toast(errorText(e), { kind: 'error' });
    } finally {
      loaded = true;
    }
  }

  onMount(() => void load());
  let firstToggle = true;
  $effect(() => {
    void showRevoked;
    if (firstToggle) firstToggle = false;
    else void load();
  });

  async function create(e: SubmitEvent) {
    e.preventDefault();
    formError = '';
    if (!name.trim()) {
      formError = 'Give the key a name, so you can tell it apart later.';
      return;
    }
    const limit = rate.trim() === '' ? null : Number(rate);
    if (limit !== null && (!Number.isInteger(limit) || limit < 1)) {
      formError = 'The rate limit is a whole number of requests per minute.';
      return;
    }
    creating = true;
    try {
      const made = await api.createKey({
        name: name.trim(),
        preset,
        deny_tags: parseTags(hidden),
        rate_limit: limit,
        expires_in_days: expiry === '0' ? undefined : Number(expiry),
      });
      secret = { name: made.name, key: made.key };
      name = '';
      hidden = '';
      rate = '';
      expiry = '0';
      await load();
    } catch (err) {
      formError = errorText(err);
    } finally {
      creating = false;
    }
  }

  function edit(k: ApiKey) {
    editing = { id: k.id, name: k.name, preset: presetId(k.scopes) ?? 'read_only', hidden: k.deny_tags.map((t) => '#' + t).join(' '), error: '', busy: false };
  }

  async function saveEdit(e: SubmitEvent) {
    e.preventDefault();
    if (!editing) return;
    const ed = editing;
    ed.busy = true;
    ed.error = '';
    const current = keys.find((k) => k.id === ed.id);
    try {
      await api.updateKey(ed.id, {
        name: ed.name.trim(),
        // Admin keys and keys with custom scopes keep theirs unless a preset is chosen on purpose.
        ...(current && presetId(current.scopes) !== null && presetId(current.scopes) !== ed.preset ? { preset: ed.preset } : {}),
        deny_tags: parseTags(ed.hidden),
      });
      editing = null;
      toast('Key updated');
      await load();
    } catch (err) {
      ed.busy = false;
      ed.error = errorText(err);
    }
  }

  async function revoke(k: ApiKey) {
    try {
      await api.revokeKey(k.id);
      revoking = null;
      toast(`${k.name} revoked`);
      await load();
    } catch (err) {
      toast(errorText(err), { kind: 'error' });
    }
  }

  const when = (iso: string | null) =>
    iso ? new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' }) : null;
  const ago = (iso: string | null) => {
    if (!iso) return 'never used';
    const m = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 60000));
    if (m < 2) return 'used just now';
    if (m < 90) return `used ${m} min ago`;
    if (m < 60 * 36) return `used ${Math.round(m / 60)} h ago`;
    return `used ${when(iso)}`;
  };
  const expired = (k: ApiKey) => k.expires_at !== null && new Date(k.expires_at).getTime() < Date.now();
  const curl = (key: string) => `curl -H "Authorization: Bearer ${key}" ${location.origin}/api/v1/gifs?limit=5`;
</script>

<section class="page">
  <h1>API keys</h1>
  <p class="lead">
    Keys let other apps and people use your library. Give each its own key and its own limits, and revoke one without touching the rest.
    The <a href="/api/docs" target="_blank" rel="noreferrer noopener">API reference</a> lists what each can call.
  </p>

  {#if denied || (app.me && !app.me.scopes.includes('admin'))}
    <p class="warn" role="note">Only the owner's sign-in, or a key with the admin scope, can manage keys.</p>
  {:else}
    {#if secret}
      <div class="card secret" role="status" data-testid="new-key">
        <h2>Copy the key for {secret.name} now</h2>
        <p>This is the only time it is shown. It cannot be recovered; if it is lost, make a new key.</p>
        <code class="key" data-testid="new-key-value">{secret.key}</code>
        <div class="buttons">
          <button class="btn primary" type="button" onclick={() => secret && copyText('Key', secret.key)}>Copy key</button>
          <button class="btn" type="button" onclick={() => secret && copyText('curl example', curl(secret.key))}>Copy curl example</button>
          <button class="btn" type="button" onclick={() => (secret = null)}>I have saved it</button>
        </div>
      </div>
    {/if}

    <form class="card" onsubmit={create}>
      <h2>New key</h2>
      <label>
        Name
        <input bind:value={name} maxlength="100" placeholder="Alex (read only)" autocomplete="off" />
      </label>
      <fieldset>
        <legend>Access</legend>
        {#each presets as p (p.id)}
          <label class="choice">
            <input type="radio" name="preset" value={p.id} bind:group={preset} />
            <span><strong>{p.label}</strong> <span class="help">{p.help}</span></span>
          </label>
        {/each}
      </fieldset>
      <label>
        Hidden hashtags <span class="help">(optional)</span>
        <input class="tag" bind:value={hidden} placeholder="#private #family" autocomplete="off" autocapitalize="off" spellcheck="false" />
        <span class="help">GIFs with any of these are invisible to the key: not listed, not found by ID, not counted.</span>
      </label>
      <div class="two">
        <label>
          Expires
          <select bind:value={expiry}>
            <option value="0">Never</option>
            <option value="7">In 7 days</option>
            <option value="30">In 30 days</option>
            <option value="90">In 90 days</option>
            <option value="365">In a year</option>
          </select>
        </label>
        <label>
          Requests a minute <span class="help">(optional)</span>
          <input bind:value={rate} inputmode="numeric" placeholder="default" autocomplete="off" />
        </label>
      </div>
      {#if formError}<p class="error" role="alert">{formError}</p>{/if}
      <div class="buttons"><button class="btn primary" type="submit" disabled={creating}>Create key</button></div>
    </form>

    <div class="head">
      <h2>Keys</h2>
      <label class="toggle"><input type="checkbox" bind:checked={showRevoked} /> Show revoked</label>
    </div>

    {#if loaded && keys.length === 0}
      <p class="lead">{showRevoked ? 'No keys yet.' : 'No active keys. Make one above.'}</p>
    {/if}

    <ul class="keys" data-testid="key-list">
      {#each keys as k (k.id)}
        <li class:revoked={k.revoked_at !== null}>
          <div class="row">
            <div class="who">
              <strong>{k.name}</strong>
              <span class="meta">
                {presetOf(k.scopes)} · {ago(k.last_used_at)} · created {when(k.created_at)}
                {#if k.revoked_at}· revoked {when(k.revoked_at)}
                {:else if k.expires_at}· {expired(k) ? 'expired' : 'expires'} {when(k.expires_at)}{/if}
                {#if k.rate_limit}· {k.rate_limit}/min{/if}
              </span>
              {#if k.deny_tags.length}
                <span class="meta">Hides <span class="tag">{k.deny_tags.map((t) => '#' + t).join(' ')}</span></span>
              {/if}
              <span class="meta id">id {k.id}</span>
            </div>
            {#if !k.revoked_at}
              <span class="acts">
                <button class="btn" type="button" aria-label="Edit {k.name}" onclick={() => edit(k)}>Edit</button>
                <button class="btn danger" type="button" aria-label="Revoke {k.name}" onclick={() => (revoking = k.id)}>Revoke</button>
              </span>
            {/if}
          </div>
          {#if revoking === k.id}
            <div class="edit">
              <p>Revoke <strong>{k.name}</strong>? Anything using it stops working immediately.</p>
              <div class="buttons">
                <button class="btn danger" type="button" onclick={() => revoke(k)}>Revoke key</button>
                <button class="btn" type="button" onclick={() => (revoking = null)}>Cancel</button>
              </div>
            </div>
          {/if}
          {#if editing && editing.id === k.id}
            <form class="edit" onsubmit={saveEdit}>
              <label>
                Name
                <input bind:value={editing.name} maxlength="100" aria-label="Key name" />
              </label>
              {#if presetId(k.scopes)}
                <label>
                  Access
                  <select bind:value={editing.preset} aria-label="Key access">
                    {#each presets as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
                  </select>
                </label>
              {/if}
              <label>
                Hidden hashtags
                <input class="tag" bind:value={editing.hidden} aria-label="Hidden hashtags" autocomplete="off" autocapitalize="off" spellcheck="false" />
              </label>
              {#if editing.error}<p class="error" role="alert">{editing.error}</p>{/if}
              <div class="buttons">
                <button class="btn primary" type="submit" disabled={editing.busy}>Save</button>
                <button class="btn" type="button" onclick={() => (editing = null)}>Cancel</button>
              </div>
            </form>
          {/if}
        </li>
      {/each}
    </ul>
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
    gap: 16px;
  }
  .page > * {
    max-width: 640px;
  }
  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }
  h2 {
    margin: 0;
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .lead {
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
  .card {
    display: grid;
    gap: 12px;
    padding: 16px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-tile);
  }
  .card p {
    margin: 0;
    color: var(--text-muted);
  }
  .secret {
    border-color: var(--accent);
  }
  .secret h2 {
    color: var(--text);
    text-transform: none;
    letter-spacing: 0;
    font-size: 15px;
  }
  .key {
    display: block;
    padding: 10px 12px;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    font: 13px var(--font-tag);
    overflow-wrap: anywhere;
    user-select: all;
  }
  label {
    display: grid;
    gap: 4px;
    color: var(--text-muted);
  }
  fieldset {
    display: grid;
    gap: 8px;
    margin: 0;
    padding: 0;
    border: 0;
  }
  legend {
    padding: 0;
    margin-bottom: 4px;
    color: var(--text-muted);
  }
  .choice {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    color: var(--text);
  }
  .choice input {
    margin-top: 4px;
  }
  .help {
    font-size: 12px;
    color: var(--text-faint);
  }
  input:not([type='radio']):not([type='checkbox']),
  select {
    min-height: 36px;
    padding: 0 10px;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    color: var(--text);
    font-size: 16px;
  }
  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .error {
    color: var(--danger) !important;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-muted);
  }
  .keys {
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
  li.revoked {
    opacity: 0.55;
  }
  .row {
    display: flex;
    gap: 12px;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
  }
  .who {
    display: grid;
    min-width: 0;
  }
  .meta {
    font-size: 12px;
    color: var(--text-muted);
  }
  .id {
    color: var(--text-faint);
    font-family: var(--font-tag);
  }
  .acts {
    display: flex;
    gap: 6px;
    flex: none;
  }
  .acts .btn {
    min-height: 32px;
    padding: 0 10px;
    font-size: 13px;
  }
  .edit {
    display: grid;
    gap: 10px;
    padding: 8px 12px 12px;
    background: var(--surface-1);
  }
  .edit p {
    margin: 0;
  }
  @media (max-width: 639px) {
    .page {
      height: calc(100dvh - var(--tabbar-h));
      padding: calc(12px + env(safe-area-inset-top)) 12px 12px;
    }
    .row {
      flex-direction: column;
      align-items: stretch;
    }
    .acts .btn {
      flex: 1;
      min-height: 44px;
    }
    .two {
      grid-template-columns: 1fr;
    }
  }
</style>

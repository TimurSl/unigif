<script lang="ts">
  import { app, logout } from '../lib/app.svelte';
  import { install, isIos, pwa } from '../lib/pwa.svelte';

  const size = (n: number) =>
    n >= 1073741824 ? `${(n / 1073741824).toFixed(1)} GB` : n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(0, Math.round(n / 1024))} kB`;
</script>

<section class="page">
  <h1>Settings</h1>

  <div class="card">
    <h2>Account</h2>
    <p>Signed in as <strong>{app.me?.name}</strong></p>
    <button class="btn danger" type="button" onclick={logout}>Sign out</button>
  </div>

  {#if app.stats}
    <div class="card">
      <h2>Storage</h2>
      <dl>
        <dt>GIFs</dt><dd>{app.stats.gifs}</dd>
        <dt>Stored</dt><dd>{size(app.stats.bytes_stored)}</dd>
        <dt>Saved by conversion</dt><dd>{size(app.stats.bytes_saved)}</dd>
      </dl>
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

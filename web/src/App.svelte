<script lang="ts">
  import { onMount } from 'svelte';
  import { app, boot, toast } from './lib/app.svelte';
  import { navigate, route } from './lib/router.svelte';
  import { takeShared } from './lib/share';
  import { looksLikeUrl, uploadFiles, uploadShared, uploadUrl } from './lib/upload';
  import AddSheet from './components/AddSheet.svelte';
  import Detail from './components/Detail.svelte';
  import ListView from './components/ListView.svelte';
  import Login from './components/Login.svelte';
  import Search from './components/Search.svelte';
  import Settings from './components/Settings.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import Toasts from './components/Toasts.svelte';

  let picker: HTMLInputElement | undefined = $state();
  let dragging = $state(false);
  let adding = $state(false);
  let depth = 0;

  onMount(() => void boot());

  // Something shared into the installed app (Android share target) waits in the worker's stash until
  // we are signed in; then it is uploaded and the app lands in Unsorted.
  let importing = false;
  $effect(() => {
    if (app.phase !== 'ready' || importing) return;
    importing = true;
    void importShared().finally(() => (importing = false));
  });

  async function importShared() {
    const arrived = location.pathname === '/share-received';
    const shared = await takeShared().catch(() => null);
    if (!shared || (shared.files.length === 0 && shared.urls.length === 0 && !shared.ignoredText)) {
      if (arrived) navigate('/', { replace: true });
      return;
    }
    navigate('/unsorted', { replace: true });
    if (shared.files.length || shared.urls.length) uploadShared(shared.files, shared.urls);
    else toast('Nothing to add: the shared text had no link in it.', { kind: 'error' });
  }

  const hasFiles = (e: DragEvent) => e.dataTransfer?.types.includes('Files') ?? false;

  function onPaste(e: ClipboardEvent) {
    if (app.phase !== 'ready') return;
    const files = Array.from(e.clipboardData?.files ?? []);
    const t = e.target as HTMLElement | null;
    const typing = t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
    if (files.length) {
      e.preventDefault();
      uploadFiles(files);
      return;
    }
    const text = e.clipboardData?.getData('text') ?? '';
    if (!typing && looksLikeUrl(text)) {
      e.preventDefault();
      uploadUrl(text.trim());
    }
  }
</script>

<svelte:window
  onpaste={onPaste}
  ondragenter={(e) => {
    if (app.phase === 'ready' && hasFiles(e)) {
      depth++;
      dragging = true;
    }
  }}
  ondragleave={() => {
    depth = Math.max(0, depth - 1);
    if (depth === 0) dragging = false;
  }}
  ondragover={(e) => hasFiles(e) && e.preventDefault()}
  ondrop={(e) => {
    depth = 0;
    dragging = false;
    if (app.phase === 'ready' && hasFiles(e)) {
      e.preventDefault();
      uploadFiles(Array.from(e.dataTransfer?.files ?? []));
    }
  }}
/>

{#if app.phase === 'loading'}
  <p class="boot">unigif</p>
{:else if app.phase === 'anon' || route.view === 'login'}
  <Login />
{:else}
  <div class="shell">
    <Sidebar onadd={() => (adding = true)} />
    {#key route.view}
      {#if route.view === 'settings'}
        <Settings />
      {:else if route.view === 'search'}
        <Search />
      {:else}
        <ListView view={route.view === 'unsorted' ? 'unsorted' : 'library'} onupload={() => picker?.click()} />
      {/if}
    {/key}
  </div>
  {#if route.gif}
    <Detail id={route.gif} />
  {/if}
  <input
    bind:this={picker}
    type="file"
    accept="image/*,video/*"
    multiple
    hidden
    data-testid="file-input"
    onchange={(e) => {
      uploadFiles(Array.from(e.currentTarget.files ?? []));
      e.currentTarget.value = '';
    }}
  />
  {#if adding}
    <AddSheet onpick={() => picker?.click()} onclose={() => (adding = false)} />
  {/if}
  {#if app.upload}
    <p class="uploading" role="status">Uploading {Math.min(app.upload.done + 1, app.upload.total)} of {app.upload.total}…</p>
  {/if}
  {#if dragging}
    <div class="drop" aria-hidden="true">Drop to add</div>
  {/if}
{/if}
<Toasts />

<style>
  .boot {
    min-height: 100dvh;
    display: grid;
    place-content: center;
    margin: 0;
    color: var(--text-faint);
    font-size: 20px;
  }
  .shell {
    display: flex;
    min-height: 100dvh;
  }
  .uploading {
    position: fixed;
    z-index: 60;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    margin: 0;
    padding: 6px 14px;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: 999px;
  }
  .drop {
    position: fixed;
    inset: 12px;
    z-index: 90;
    display: grid;
    place-content: center;
    border: 2px dashed var(--text-muted);
    border-radius: var(--radius-tile);
    background: rgb(10 10 10 / 0.85);
    font-size: 20px;
    pointer-events: none;
  }
</style>

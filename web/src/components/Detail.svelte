<script lang="ts">
  import { onMount } from 'svelte';
  import { api, mediaPath, type Gif } from '../lib/api';
  import { emitGif, errorText, onGifEvent, refreshStats, toast } from '../lib/app.svelte';
  import { closeGif, navigate, href } from '../lib/router.svelte';
  import Sorter from './Sorter.svelte';

  let { id }: { id: string } = $props();

  let gif = $state<Gif | null>(null);
  let failed = $state('');
  let editing = $state(false);
  let dialog: HTMLDivElement | undefined = $state();
  const opener = document.activeElement as HTMLElement | null;

  $effect(() => {
    const want = id;
    gif = null;
    failed = '';
    api
      .getGif(want)
      .then((g) => {
        if (want === id) gif = g;
      })
      .catch((e) => (failed = errorText(e)));
  });

  onMount(() => {
    dialog?.focus();
    return () => opener?.focus?.();
  });

  // Keep the detail view in step with edits made through the tag sheet.
  onMount(() =>
    onGifEvent((kind, g) => {
      if (kind === 'changed' && g.id === id) gif = g;
    }),
  );

  const src = $derived(gif?.media?.mp4 ? mediaPath(gif.media.mp4.url) : null);
  const absolute = (p: string) => new URL(p, location.href).href;

  async function copy(label: string, text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast(`${label} copied`);
    } catch {
      toast('Could not copy', { kind: 'error' });
    }
  }

  function showTag(tag: string) {
    closeGif();
    setTimeout(() => navigate(href('library', tag)), 0);
  }

  async function trash() {
    if (!gif) return;
    const g = gif;
    try {
      await api.trash(g.id);
    } catch (e) {
      toast(errorText(e), { kind: 'error' });
      return;
    }
    emitGif('removed', g);
    void refreshStats();
    closeGif();
    toast('Moved to trash', {
      action: {
        label: 'Undo',
        run: () =>
          api
            .restore(g.id)
            .then((r) => {
              emitGif('added', r);
              void refreshStats();
            })
            .catch((e) => toast(errorText(e), { kind: 'error' })),
      },
      ms: 5000,
    });
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && !editing) {
      e.preventDefault();
      closeGif();
    }
  }

  const size = (n: number) => (n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} kB`);
</script>

<svelte:window onkeydown={onKey} />

<div class="scrim" role="presentation" onmousedown={(e) => e.target === e.currentTarget && closeGif()}>
  <div class="detail" role="dialog" aria-modal="true" aria-label="GIF details" tabindex="-1" bind:this={dialog}>
    <header>
      <h2>GIF</h2>
      <button class="btn ghost" type="button" aria-label="Close" onclick={closeGif}><kbd>Esc</kbd> ✕</button>
    </header>
    {#if failed}
      <p class="status">{failed}</p>
    {:else if !gif}
      <p class="status">Loading…</p>
    {:else}
      <div class="body">
        <div class="player" style:aspect-ratio={gif.width && gif.height ? `${gif.width} / ${gif.height}` : undefined}>
          {#if src}
            <video {src} autoplay muted loop playsinline controls></video>
          {:else}
            <p class="status">{gif.status === 'failed' ? 'Could not be processed' : 'Processing…'}</p>
          {/if}
        </div>
        <div class="side">
          <section>
            <div class="row">
              <h3>Hashtags</h3>
              <button class="btn" type="button" onclick={() => (editing = true)}>Edit hashtags</button>
            </div>
            <div class="tags">
              {#each gif.tags as t (t)}
                <button class="chip tag" type="button" onclick={() => showTag(t)}>#{t}</button>
              {:else}
                <span class="muted">None yet. This GIF is in Unsorted.</span>
              {/each}
            </div>
          </section>
          <section>
            <h3>Copy</h3>
            <div class="row wrap">
              {#if gif.media?.mp4}
                <button class="btn" type="button" onclick={() => copy('Link', absolute(mediaPath(gif!.media!.mp4!.url)))}>Link</button>
                <button class="btn" type="button" onclick={() => copy('MP4 link', absolute(mediaPath(gif!.media!.mp4!.url)))}>MP4</button>
              {/if}
            </div>
          </section>
          <section>
            <h3>Info</h3>
            <dl>
              {#if gif.width && gif.height}<dt>Size</dt><dd>{gif.width} × {gif.height}</dd>{/if}
              {#if gif.duration_ms}<dt>Length</dt><dd>{(gif.duration_ms / 1000).toFixed(1)} s</dd>{/if}
              {#if gif.media?.mp4}<dt>Stored</dt><dd>{size(gif.media.mp4.bytes)}</dd>{/if}
              <dt>Source</dt><dd>{gif.source.type}</dd>
              <dt>Added</dt><dd>{new Date(gif.created_at).toLocaleString()}</dd>
            </dl>
          </section>
          <div class="danger-row">
            <button class="btn danger" type="button" onclick={trash}>Delete</button>
          </div>
        </div>
      </div>
    {/if}
  </div>
</div>

{#if editing && gif}
  <Sorter mode="edit" {gif} onclose={() => (editing = false)} />
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgb(0 0 0 / 0.72);
  }
  .detail {
    display: flex;
    flex-direction: column;
    width: min(1040px, 100%);
    max-height: 100%;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-tile);
    outline: none;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 8px 8px 16px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-muted);
  }
  h3 {
    margin: 0 0 8px;
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .status {
    padding: 40px;
    text-align: center;
    color: var(--text-muted);
  }
  .body {
    display: grid;
    grid-template-columns: minmax(0, 1.3fr) minmax(260px, 1fr);
    min-height: 0;
    overflow-y: auto;
  }
  .player {
    display: grid;
    place-items: center;
    padding: 16px;
    background: #000;
    min-height: 220px;
    max-height: 70vh;
  }
  .player video {
    max-width: 100%;
    max-height: 100%;
    border-radius: 6px;
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding: 16px;
    border-left: 1px solid var(--border);
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
  }
  .row h3 {
    margin: 0;
  }
  .row.wrap {
    justify-content: flex-start;
    flex-wrap: wrap;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 10px;
  }
  .chip {
    min-height: 32px;
    padding: 0 10px;
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    background: transparent;
    font-size: 13px;
  }
  .chip:hover {
    background: var(--surface-2);
  }
  .muted {
    color: var(--text-faint);
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
  .danger-row {
    margin-top: auto;
  }
  @media (max-width: 720px) {
    .scrim {
      padding: 0;
    }
    .detail {
      height: 100%;
      border-radius: 0;
      border: 0;
    }
    .body {
      grid-template-columns: 1fr;
    }
    .side {
      border-left: 0;
      border-top: 1px solid var(--border);
    }
  }
</style>

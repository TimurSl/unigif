<script lang="ts">
  import { thumbHashToDataURL } from 'thumbhash';
  import { mediaPath, type Gif } from '../lib/api';
  import { toast } from '../lib/app.svelte';

  let { gif, onopen }: { gif: Gif; onopen: (g: Gif) => void } = $props();

  const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;

  let el: HTMLButtonElement | undefined = $state();
  let video: HTMLVideoElement | undefined = $state();
  let visible = $state(false);
  let hovering = $state(false);
  let ready = $state(false);

  const placeholder = $derived.by(() => {
    if (!gif.thumbhash) return '';
    try {
      const bin = atob(gif.thumbhash);
      return thumbHashToDataURL(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
    } catch {
      return '';
    }
  });

  const preview = $derived(gif.media?.preview_mp4 ? mediaPath(gif.media.preview_mp4.url) : null);
  const poster = $derived(gif.media?.poster ? mediaPath(gif.media.poster.url) : null);
  const play = $derived(visible && preview !== null && (!reduceMotion || hovering));

  $effect(() => {
    if (!el) return;
    const io = new IntersectionObserver(([entry]) => (visible = entry.isIntersecting));
    io.observe(el);
    return () => io.disconnect();
  });

  // Off-screen videos are paused and unloaded.
  $effect(() => {
    if (!video) return;
    if (play) {
      void video.play().catch(() => {});
    } else {
      video.pause();
    }
  });

  async function copyLink(e: MouseEvent | KeyboardEvent) {
    e.stopPropagation();
    const url = gif.media?.mp4 ? new URL(mediaPath(gif.media.mp4.url), location.href).href : location.href;
    try {
      await navigator.clipboard.writeText(url);
      toast('Link copied');
    } catch {
      toast('Could not copy the link', { kind: 'error' });
    }
  }
</script>

<div class="tile" role="presentation" onmouseenter={() => (hovering = true)} onmouseleave={() => (hovering = false)}>
  <button
    bind:this={el}
    class="open"
    type="button"
    style:background-image={placeholder ? `url(${placeholder})` : undefined}
    aria-label={gif.tags.length ? `GIF tagged ${gif.tags.map((t) => '#' + t).join(' ')}` : 'GIF without hashtags'}
    onclick={() => onopen(gif)}
  >
    {#if gif.status === 'failed'}
      <span class="note">Could not be processed</span>
    {:else if gif.status === 'processing' || !preview}
      <span class="note">Processing…</span>
    {:else}
      {#if poster && !ready}
        <img class="media" src={poster} alt="" draggable="false" />
      {/if}
      {#if visible}
        <video
          bind:this={video}
          class="media"
          class:hidden={!ready}
          src={preview}
          muted
          loop
          playsinline
          preload="auto"
          disablepictureinpicture
          onloadeddata={() => (ready = true)}
        ></video>
      {/if}
    {/if}
  </button>
  {#if gif.tags.length}
    <div class="meta">
      <span class="tags tag">{gif.tags.map((t) => '#' + t).join(' ')}</span>
    </div>
  {/if}
  <button class="copy" type="button" aria-label="Copy link" onclick={copyLink}>
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="9" width="12" height="12" rx="2" /><path d="M5 15V5a2 2 0 0 1 2-2h10" /></svg>
  </button>
</div>

<style>
  .tile {
    position: relative;
    width: 100%;
    height: 100%;
    border-radius: var(--radius-tile);
    overflow: hidden;
    background: var(--surface-1);
    border: 1px solid var(--border);
  }
  .open {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    padding: 0;
    border: 0;
    background-color: var(--surface-1);
    background-size: cover;
    background-position: center;
    display: grid;
    place-items: center;
  }
  .media {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .media.hidden {
    opacity: 0;
  }
  .note {
    position: relative;
    font-size: 13px;
    color: var(--text-muted);
    background: rgb(10 10 10 / 0.7);
    padding: 2px 8px;
    border-radius: 999px;
  }
  .meta {
    position: absolute;
    inset: auto 0 0 0;
    padding: 24px 8px 6px;
    background: linear-gradient(transparent, rgb(0 0 0 / 0.8));
    font-size: 12px;
    opacity: 0;
    transition: opacity var(--motion);
    pointer-events: none;
  }
  .tags {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .copy {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    background: rgb(10 10 10 / 0.8);
    opacity: 0;
    transition: opacity var(--motion);
  }
  .tile:hover .meta,
  .tile:hover .copy,
  .tile:focus-within .meta,
  .tile:focus-within .copy {
    opacity: 1;
  }
  @media (hover: none) {
    .copy {
      display: none;
    }
  }
</style>

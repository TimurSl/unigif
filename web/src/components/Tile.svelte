<script lang="ts">
  import { thumbHashToDataURL } from 'thumbhash';
  import { copyGif, gifUrl } from '../lib/actions';
  import { mediaPath, type Gif } from '../lib/api';

  let {
    gif,
    onopen,
    onmenu,
    animate = true,
    selecting = false,
    selected = false,
  }: {
    gif: Gif;
    onopen: (g: Gif, e?: MouseEvent) => void;
    /** Select mode: the tile shows a tick and a click toggles it. */
    selecting?: boolean;
    selected?: boolean;
    /** Long-press (touch). */
    onmenu?: (g: Gif) => void;
    /** False when the grid has used up its animation budget (phones): the tile shows its poster. */
    animate?: boolean;
  } = $props();

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
  // Reduced motion on a touch screen has no hover to preview with: the first tap plays, the second opens.
  let tapped = $state(false);
  const play = $derived(visible && preview !== null && (tapped || (animate && (!reduceMotion || hovering))));
  $effect(() => {
    if (!visible) tapped = false;
  });

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

  function copyLink(e: MouseEvent | KeyboardEvent) {
    e.stopPropagation();
    void copyGif(gif, 'Link', gifUrl(gif) ?? location.href);
  }

  // Long-press (touch and pen only; a mouse has hover and right-click) opens the tile menu. A scroll or
  // a drag past a few pixels cancels it, and the click that follows a long press must not open the tile.
  const LONG_PRESS_MS = 450;
  const MOVE_SLOP = 10;
  let pressTimer: ReturnType<typeof setTimeout> | undefined;
  let pressAt = { x: 0, y: 0 };
  let pressedLong = false;
  let touching = false;

  function pressStart(e: PointerEvent) {
    touching = e.pointerType !== 'mouse';
    if (!touching || !onmenu) return;
    pressedLong = false;
    pressAt = { x: e.clientX, y: e.clientY };
    clearTimeout(pressTimer);
    pressTimer = setTimeout(() => {
      pressedLong = true;
      navigator.vibrate?.(10);
      onmenu(gif);
    }, LONG_PRESS_MS);
  }
  function pressMove(e: PointerEvent) {
    if (pressTimer && Math.hypot(e.clientX - pressAt.x, e.clientY - pressAt.y) > MOVE_SLOP) pressEnd();
  }
  function pressEnd() {
    clearTimeout(pressTimer);
    pressTimer = undefined;
  }
  function click(e: MouseEvent) {
    if (pressedLong) {
      pressedLong = false;
      return;
    }
    if (reduceMotion && touching && !selecting && preview && !tapped) {
      tapped = true;
      return;
    }
    onopen(gif, e);
  }

  $effect(() => () => clearTimeout(pressTimer));

  // The description is the alt text when there is one; the hashtags follow it.
  const label = $derived.by(() => {
    const tags = gif.tags.length ? `tagged ${gif.tags.map((t) => '#' + t).join(' ')}` : 'without hashtags';
    return gif.description ? `${gif.description}, ${tags}` : `GIF ${tags}`;
  });
</script>

<div class="tile" class:selected role="presentation" onmouseenter={() => (hovering = !touching)} onmouseleave={() => (hovering = false)}>
  <button
    bind:this={el}
    class="open"
    type="button"
    style:background-image={placeholder ? `url(${placeholder})` : undefined}
    aria-label={label}
    aria-pressed={selecting ? selected : undefined}
    onclick={click}
    onpointerdown={pressStart}
    onpointermove={pressMove}
    onpointerup={pressEnd}
    onpointercancel={pressEnd}
    oncontextmenu={(e) => touching && e.preventDefault()}
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
  {#if selecting}
    <span class="tick" class:on={selected} aria-hidden="true">
      {#if selected}<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><path d="m5 12 5 5 9-10" /></svg>{/if}
    </span>
  {:else}
  <button class="copy" type="button" aria-label="Copy link" onclick={copyLink}>
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="9" width="12" height="12" rx="2" /><path d="M5 15V5a2 2 0 0 1 2-2h10" /></svg>
  </button>
  {/if}
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
    -webkit-touch-callout: none;
    -webkit-user-select: none;
    user-select: none;
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
  .tile.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent);
  }
  .tick {
    position: absolute;
    top: 6px;
    left: 6px;
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    border: 2px solid var(--text);
    border-radius: 999px;
    background: rgb(10 10 10 / 0.6);
    pointer-events: none;
  }
  .tick.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
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
    /* No hover on touch: the hashtags are always shown. */
    .meta {
      opacity: 1;
    }
  }
</style>

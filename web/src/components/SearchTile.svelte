<script lang="ts">
  import type { SearchItem } from '../lib/api';

  let {
    item,
    status,
    onadd,
  }: {
    item: SearchItem;
    /** `idle`, `adding`, `added` (new in the library) or `have` (it was already there). */
    status: 'idle' | 'adding' | 'added' | 'have';
    onadd: (item: SearchItem) => void;
  } = $props();

  const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;

  let el: HTMLDivElement | undefined = $state();
  let visible = $state(false);
  let hovering = $state(false);

  const label = $derived(
    status === 'added' ? 'Added' : status === 'have' ? 'In library' : status === 'adding' ? 'Adding…' : 'Add',
  );
  const play = $derived(visible && (!reduceMotion || hovering));
  let video: HTMLVideoElement | undefined = $state();

  $effect(() => {
    if (!video) return;
    if (play) void video.play().catch(() => {});
    else video.pause();
  });

  // Only tiles on screen load media, so a long scroll never holds hundreds of videos open.
  $effect(() => {
    if (!el) return;
    const io = new IntersectionObserver(([entry]) => (visible = entry.isIntersecting), { rootMargin: '200px' });
    io.observe(el);
    return () => io.disconnect();
  });
</script>

<div
  bind:this={el}
  class="tile"
  role="presentation"
  style:aspect-ratio={item.width && item.height ? `${item.width} / ${item.height}` : '1'}
  onmouseenter={() => (hovering = true)}
  onmouseleave={() => (hovering = false)}
>
  {#if visible}
    {#if item.preview.kind === 'video'}
      <video bind:this={video} src={item.preview.url} muted loop playsinline preload="auto" disablepictureinpicture></video>
    {:else}
      <img src={item.preview.url} alt="" draggable="false" loading="lazy" referrerpolicy="no-referrer" />
    {/if}
  {/if}
  <button
    class="add"
    class:done={status === 'added' || status === 'have'}
    type="button"
    disabled={status !== 'idle'}
    aria-label={item.title ? `${label}: ${item.title}` : label}
    data-state={status}
    onclick={() => onadd(item)}
  >
    <span class="pill">
      {#if status === 'added' || status === 'have'}
        ✓
      {:else if status === 'adding'}
        …
      {:else}
        +
      {/if}
      <span class="word">{label}</span>
    </span>
  </button>
  {#if item.title}
    <p class="title">{item.title}</p>
  {/if}
</div>

<style>
  .tile {
    position: relative;
    overflow: hidden;
    border-radius: var(--radius-tile);
    background: var(--surface-1);
    border: 1px solid var(--border);
    break-inside: avoid;
    margin-bottom: 12px;
  }
  video,
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .add {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: flex-end;
    justify-content: flex-end;
    padding: 8px;
    border: 0;
    background: transparent;
    color: var(--text);
  }
  .add:disabled {
    cursor: default;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 32px;
    padding: 0 12px;
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    background: rgb(10 10 10 / 0.85);
    font-size: 13px;
    opacity: 0;
    transition: opacity var(--motion);
  }
  .tile:hover .pill,
  .add:focus-visible .pill,
  .add.done .pill,
  .add[data-state='adding'] .pill {
    opacity: 1;
  }
  .add.done .pill {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .title {
    position: absolute;
    inset: auto 0 0 0;
    margin: 0;
    padding: 24px 90px 6px 8px;
    background: linear-gradient(transparent, rgb(0 0 0 / 0.8));
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    opacity: 0;
    transition: opacity var(--motion);
    pointer-events: none;
  }
  .tile:hover .title {
    opacity: 1;
  }
  @media (hover: none) {
    /* No hover on touch: the button is always there, and big enough to tap. */
    .pill {
      opacity: 1;
      min-height: 44px;
      padding: 0 16px;
    }
    .title {
      display: none;
    }
  }
</style>

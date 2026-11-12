<script lang="ts">
  import type { Gif } from '../lib/api';
  import Tile from './Tile.svelte';

  let {
    items,
    loading,
    done,
    onopen,
    onend,
    onmenu,
    empty,
  }: {
    items: Gif[];
    loading: boolean;
    done: boolean;
    onopen: (g: Gif) => void;
    onend: () => void;
    /** Long-press on a tile (touch only). */
    onmenu?: (g: Gif) => void;
    empty?: import('svelte').Snippet;
  } = $props();

  const GAP = 12;
  /** On a phone at most this many tiles animate at once; the rest show posters (data use, battery). */
  const PHONE_MAX_PLAYING = 6;
  const MIN_COL = 220;
  const BUFFER = 600;

  const phoneQuery = matchMedia('(max-width: 639px)');
  let phone = $state(phoneQuery.matches);
  $effect(() => {
    const on = () => (phone = phoneQuery.matches);
    phoneQuery.addEventListener('change', on);
    return () => phoneQuery.removeEventListener('change', on);
  });

  let width = $state(0);
  let height = $state(0);
  let scrollTop = $state(0);
  let scroller: HTMLDivElement | undefined = $state();

  const cols = $derived(width < 640 ? 2 : Math.max(2, Math.floor((width + GAP) / (MIN_COL + GAP))));
  const colW = $derived(width > 0 ? (width - GAP * (cols - 1)) / cols : 0);

  // Masonry placement from stored dimensions, so no tile ever shifts when its media loads.
  const layout = $derived.by(() => {
    const heights = new Array<number>(cols).fill(0);
    const boxes: { x: number; y: number; h: number }[] = [];
    if (colW <= 0) return { boxes, total: 0 };
    for (const g of items) {
      const ratio = g.width && g.height ? Math.min(2.5, Math.max(0.4, g.height / g.width)) : 1;
      const h = Math.round(colW * ratio);
      let c = 0;
      for (let i = 1; i < cols; i++) if (heights[i] < heights[c]) c = i;
      boxes.push({ x: c * (colW + GAP), y: heights[c], h });
      heights[c] += h + GAP;
    }
    return { boxes, total: Math.max(0, ...heights) };
  });

  // Only tiles near the viewport exist in the DOM, so tens of thousands of GIFs stay smooth.
  const visible = $derived.by(() => {
    const out: number[] = [];
    const top = scrollTop - BUFFER;
    const bottom = scrollTop + height + BUFFER;
    layout.boxes.forEach((b, i) => {
      if (b.y + b.h >= top && b.y <= bottom) out.push(i);
    });
    return out;
  });

  // The tiles that actually animate: those on screen, top to bottom and left to right, up to the cap.
  const playing = $derived.by(() => {
    if (!phone) return null; // no cap: every on-screen tile plays
    const onScreen = visible.filter((i) => layout.boxes[i].y + layout.boxes[i].h > scrollTop && layout.boxes[i].y < scrollTop + height);
    onScreen.sort((a, b) => layout.boxes[a].y - layout.boxes[b].y || layout.boxes[a].x - layout.boxes[b].x);
    return new Set(onScreen.slice(0, PHONE_MAX_PLAYING));
  });

  function maybeMore() {
    if (!scroller || loading || done) return;
    if (scroller.scrollTop + scroller.clientHeight > layout.total - 1200) onend();
  }

  $effect(() => {
    // Re-check after every layout change: a short list may not fill the viewport yet.
    void layout.total;
    void height;
    void loading;
    maybeMore();
  });
</script>

<div
  class="scroller"
  bind:this={scroller}
  bind:clientHeight={height}
  onscroll={() => {
    scrollTop = scroller!.scrollTop;
    maybeMore();
  }}
>
  <div class="inner" bind:clientWidth={width}>
    {#if items.length === 0 && done && !loading}
      <div class="empty">{@render empty?.()}</div>
    {:else}
    <div class="canvas" style:height="{layout.total}px">
      {#each visible as i (items[i].id)}
        {@const b = layout.boxes[i]}
        <div class="slot" style:transform="translate({b.x}px, {b.y}px)" style:width="{colW}px" style:height="{b.h}px">
          <Tile gif={items[i]} {onopen} {onmenu} animate={playing === null || playing.has(i)} />
        </div>
      {/each}
    </div>
    {#if loading}<p class="more">Loading…</p>{/if}
    {/if}
  </div>
</div>

<style>
  .scroller {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 16px 16px;
  }
  @media (max-width: 639px) {
    .scroller {
      padding: 0 12px 12px;
    }
  }
  .canvas {
    position: relative;
  }
  .slot {
    position: absolute;
    top: 0;
    left: 0;
  }
  .more {
    text-align: center;
    color: var(--text-faint);
    margin: 12px 0 0;
  }
  .empty {
    display: grid;
    place-content: center;
    min-height: 50%;
    text-align: center;
    color: var(--text-muted);
    gap: 8px;
  }
</style>

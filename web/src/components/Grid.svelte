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
    selecting = false,
    selected,
    onband,
    onbandend,
  }: {
    items: Gif[];
    loading: boolean;
    done: boolean;
    onopen: (g: Gif, e?: MouseEvent) => void;
    /** Select mode: tiles show ticks; `selected` says which are ticked. */
    selecting?: boolean;
    selected?: (id: string) => boolean;
    onend: () => void;
    /** Long-press on a tile (touch only). */
    onmenu?: (g: Gif) => void;
    empty?: import('svelte').Snippet;
    /**
     * Rubber-band selection with a mouse: called while dragging with the ids the band covers. Starts
     * anywhere in select mode, or on empty space between tiles otherwise. Touch never starts one.
     */
    onband?: (ids: string[]) => void;
    onbandend?: () => void;
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

  // ---- Rubber band -------------------------------------------------------------------------------
  const DRAG_SLOP = 6;
  let canvas: HTMLDivElement | undefined = $state();
  let press: { x: number; y: number; id: number } | null = null;
  let band = $state<{ x: number; y: number; w: number; h: number } | null>(null);
  let lastPointer = { x: 0, y: 0 };
  let suppressClick = false;

  /** A point in canvas coordinates (they scroll with the content). */
  function toCanvas(clientX: number, clientY: number) {
    const r = canvas!.getBoundingClientRect();
    return { x: clientX - r.left, y: clientY - r.top };
  }

  function bandDown(e: PointerEvent) {
    if (!onband || !canvas || e.pointerType !== 'mouse' || e.button !== 0) return;
    const onTile = (e.target as HTMLElement).closest('.slot') !== null;
    if (!selecting && onTile) return;
    press = { ...toCanvas(e.clientX, e.clientY), id: e.pointerId };
    lastPointer = { x: e.clientX, y: e.clientY };
  }

  function updateBand() {
    if (!press || !canvas) return;
    const p = toCanvas(lastPointer.x, lastPointer.y);
    const x = Math.min(press.x, p.x);
    const y = Math.min(press.y, p.y);
    const w = Math.abs(p.x - press.x);
    const h = Math.abs(p.y - press.y);
    if (!band && Math.hypot(w, h) < DRAG_SLOP) return;
    if (!band) canvas.setPointerCapture?.(press.id);
    band = { x, y, w, h };
    const ids: string[] = [];
    layout.boxes.forEach((b, i) => {
      if (b.x < x + w && b.x + colW > x && b.y < y + h && b.y + b.h > y) ids.push(items[i].id);
    });
    onband?.(ids);
  }

  function bandMove(e: PointerEvent) {
    if (!press || e.pointerId !== press.id) return;
    lastPointer = { x: e.clientX, y: e.clientY };
    updateBand();
    // Near the top or bottom edge the list scrolls, so a band can reach past the screen.
    if (band && scroller) {
      const r = scroller.getBoundingClientRect();
      if (e.clientY > r.bottom - 40) scroller.scrollBy(0, 24);
      else if (e.clientY < r.top + 40) scroller.scrollBy(0, -24);
    }
  }

  function bandUp(e: PointerEvent) {
    if (!press || e.pointerId !== press.id) return;
    if (band) {
      // The click that follows the drag must not toggle or open the tile under the pointer.
      suppressClick = true;
      setTimeout(() => (suppressClick = false), 0);
      onbandend?.();
    }
    press = null;
    band = null;
  }

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
    if (band) updateBand();
  }}
  onclickcapture={(e) => {
    if (suppressClick) {
      e.stopPropagation();
      e.preventDefault();
      suppressClick = false;
    }
  }}
>
  <div class="inner" bind:clientWidth={width}>
    {#if items.length === 0 && done && !loading}
      <div class="empty">{@render empty?.()}</div>
    {:else}
    <div
      class="canvas"
      class:banding={band !== null}
      style:height="{layout.total}px"
      role="presentation"
      bind:this={canvas}
      onpointerdown={bandDown}
      onpointermove={bandMove}
      onpointerup={bandUp}
      onpointercancel={bandUp}
    >
      {#each visible as i (items[i].id)}
        {@const b = layout.boxes[i]}
        <div class="slot" style:transform="translate({b.x}px, {b.y}px)" style:width="{colW}px" style:height="{b.h}px">
          <Tile
            gif={items[i]}
            {onopen}
            onmenu={selecting ? undefined : onmenu}
            animate={playing === null || playing.has(i)}
            {selecting}
            selected={selected?.(items[i].id) ?? false}
          />
        </div>
      {/each}
      {#if band}
        <div class="band" data-testid="rubber-band" style:transform="translate({band.x}px, {band.y}px)" style:width="{band.w}px" style:height="{band.h}px"></div>
      {/if}
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
  .canvas.banding {
    user-select: none;
  }
  .band {
    position: absolute;
    top: 0;
    left: 0;
    z-index: 5;
    border: 1px solid var(--accent);
    background: rgb(255 255 255 / 0.08);
    pointer-events: none;
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

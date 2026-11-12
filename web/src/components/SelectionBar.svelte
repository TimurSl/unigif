<script lang="ts">
  import type { Snippet } from 'svelte';

  // The bar at the bottom of a list in select mode: how many are ticked, select-all and cancel, and the actions.
  let {
    count,
    loaded,
    onall,
    oncancel,
    children,
  }: { count: number; loaded: number; onall: () => void; oncancel: () => void; children: Snippet } = $props();
</script>

<div class="bar" role="toolbar" aria-label="Selected GIFs">
  <span class="count" role="status" data-testid="selected-count">{count} selected</span>
  <button class="btn ghost" type="button" disabled={count >= loaded} onclick={onall}>Select all loaded</button>
  <span class="actions">{@render children()}</span>
  <button class="btn ghost" type="button" onclick={oncancel}>Cancel <kbd>Esc</kbd></button>
</div>

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
    padding: 8px 16px;
    background: var(--surface-1);
    border-top: 1px solid var(--border-strong);
  }
  .count {
    font-weight: 500;
    min-width: 96px;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-left: auto;
  }
  @media (max-width: 639px) {
    .bar {
      padding: 8px 12px;
    }
    .actions {
      margin-left: 0;
      width: 100%;
      order: 3;
    }
    .actions :global(.btn) {
      flex: 1;
    }
  }
</style>

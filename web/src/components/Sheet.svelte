<script lang="ts">
  import { onMount, type Snippet } from 'svelte';

  // A bottom sheet on a phone, a centred card on a desktop. Esc, the backdrop and the close button dismiss it.
  let { label, onclose, children }: { label: string; onclose: () => void; children: Snippet } = $props();

  let dialog: HTMLDivElement | undefined = $state();
  const opener = document.activeElement as HTMLElement | null;
  const openedAt = performance.now();

  onMount(() => {
    // Focus the first control so keyboard and screen-reader users start inside the sheet.
    dialog?.querySelector<HTMLElement>('button, input, a[href]')?.focus();
    return () => opener?.focus?.();
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<!-- A sheet opened by a long press appears under the finger; the click on lift-off must not dismiss it. -->
<div class="scrim" role="presentation" onclick={(e) => e.target === e.currentTarget && performance.now() - openedAt > 400 && onclose()}>
  <div class="sheet" role="dialog" aria-modal="true" aria-label={label} tabindex="-1" bind:this={dialog}>
    <div class="grab" aria-hidden="true"></div>
    <h2>{label}</h2>
    {@render children()}
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 70;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgb(0 0 0 / 0.72);
  }
  .sheet {
    display: grid;
    gap: 8px;
    width: min(420px, 100%);
    padding: 12px 16px 16px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-tile);
    outline: none;
  }
  .grab {
    display: none;
  }
  h2 {
    margin: 4px 0 4px;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-muted);
  }
  @media (max-width: 639px) {
    .scrim {
      align-items: end;
      padding: 0;
    }
    .sheet {
      width: 100%;
      padding-bottom: calc(16px + env(safe-area-inset-bottom));
      border-width: 1px 0 0;
      border-radius: 14px 14px 0 0;
    }
    .grab {
      display: block;
      width: 36px;
      height: 4px;
      margin: 0 auto 4px;
      border-radius: 2px;
      background: var(--border-strong);
    }
  }
</style>

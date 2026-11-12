<script lang="ts">
  import { app, dismissToast } from '../lib/app.svelte';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div class="toast" class:error={t.kind === 'error'}>
      <span>{t.text}</span>
      {#if t.action}
        <button
          type="button"
          onclick={() => {
            t.action!.run();
            dismissToast(t.id);
          }}>{t.action.label}</button
        >
      {/if}
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    z-index: 100;
    left: 50%;
    bottom: calc(16px + env(safe-area-inset-bottom));
    transform: translateX(-50%);
    display: grid;
    gap: 8px;
    width: min(440px, calc(100% - 32px));
    pointer-events: none;
  }
  .toast {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 16px;
    padding: 10px 14px;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    pointer-events: auto;
  }
  .toast.error {
    border-color: var(--danger);
  }
  /* Phone: at the top, so a toast never covers the tab bar or a pinned Save button. */
  @media (max-width: 639px) {
    .toasts {
      top: calc(56px + env(safe-area-inset-top));
      bottom: auto;
    }
  }
  button {
    border: 0;
    background: transparent;
    font-weight: 600;
    min-height: 32px;
    padding: 0 4px;
  }
</style>

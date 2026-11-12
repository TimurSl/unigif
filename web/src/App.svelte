<script lang="ts">
  // Milestone 0: an empty shell that proves the embedded SPA is served and can
  // reach the backend. The real UI starts in milestone 3.
  type Health = 'checking' | 'ok' | 'down';
  let health: Health = $state('checking');

  $effect(() => {
    fetch('/readyz')
      .then((r) => (health = r.ok ? 'ok' : 'down'))
      .catch(() => (health = 'down'));
  });
</script>

<main>
  <h1>unigif</h1>
  <p class="muted">Your GIF library, sorted by <span class="tag">#hashtags</span>.</p>
  <p class="status" data-health={health}>
    {#if health === 'checking'}Checking server…{:else if health === 'ok'}Server is ready{:else}Server is not reachable{/if}
  </p>
</main>

<style>
  main {
    min-height: 100dvh;
    display: grid;
    place-content: center;
    gap: 8px;
    padding: 24px;
    text-align: center;
  }
  h1 {
    margin: 0;
    font-size: 28px;
    font-weight: 600;
    letter-spacing: -0.02em;
  }
  .muted {
    margin: 0;
    color: var(--text-muted);
  }
  .tag {
    font-family: var(--font-tag);
  }
  .status {
    margin: 16px 0 0;
    font-size: 13px;
    color: var(--text-faint);
  }
  .status[data-health='ok'] {
    color: var(--text-muted);
  }
  .status[data-health='down'] {
    color: var(--danger);
  }
</style>

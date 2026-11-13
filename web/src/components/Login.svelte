<script lang="ts">
  import { login, errorText } from '../lib/app.svelte';

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = '';
    try {
      await login(username.trim(), password);
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<main>
  <form onsubmit={submit}>
    <h1>unigif</h1>
    <label>
      <span class="sr-only">Username</span>
      <input bind:value={username} name="username" placeholder="Username" autocomplete="username" autocapitalize="off" required />
    </label>
    <label>
      <span class="sr-only">Password</span>
      <input bind:value={password} name="password" type="password" placeholder="Password" autocomplete="current-password" required />
    </label>
    <p class="error" role="alert">{error}</p>
    <button class="btn primary" type="submit" disabled={busy}>{busy ? 'Signing in…' : 'Sign in'}</button>
  </form>
</main>

<style>
  main {
    min-height: 100dvh;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    place-items: center;
    padding: 24px;
  }
  form {
    display: grid;
    gap: 12px;
    width: min(320px, 100%);
  }
  h1 {
    margin: 0 0 12px;
    font-size: 28px;
    font-weight: 600;
    letter-spacing: -0.02em;
    text-align: center;
  }
  input {
    width: 100%;
    min-height: 44px;
    padding: 0 12px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    font-size: 16px;
  }
  input:focus {
    outline: none;
    border-color: var(--border-strong);
  }
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .error {
    min-height: 20px;
    margin: 0;
    color: var(--danger);
    font-size: 13px;
  }
  .btn {
    min-height: 44px;
  }
</style>

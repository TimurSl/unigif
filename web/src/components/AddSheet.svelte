<script lang="ts">
  import { toast } from '../lib/app.svelte';
  import { looksLikeUrl, uploadUrl } from '../lib/upload';
  import Sheet from './Sheet.svelte';

  let { onpick, onclose }: { onpick: () => void; onclose: () => void } = $props();

  let link = $state('');

  function addLink(e: SubmitEvent) {
    e.preventDefault();
    const url = link.trim();
    if (!looksLikeUrl(url)) {
      toast('That does not look like a link (it should start with https://).', { kind: 'error' });
      return;
    }
    uploadUrl(url);
    onclose();
  }
</script>

<Sheet label="Add GIFs" {onclose}>
  <!-- The file input lives in App; opening its picker must happen inside this click. -->
  <button
    class="btn primary big"
    type="button"
    onclick={() => {
      onpick();
      onclose();
    }}>Choose files</button
  >
  <form onsubmit={addLink}>
    <label>
      <span class="sr-only">Link to a GIF or video</span>
      <input bind:value={link} type="url" inputmode="url" placeholder="Paste a link to a GIF or video" autocomplete="off" autocapitalize="off" spellcheck="false" enterkeyhint="go" />
    </label>
    <button class="btn big" type="submit" disabled={!link.trim()}>Add link</button>
  </form>
  <p class="hint">On Android you can also share a GIF to unigif from any app, once it is installed.</p>
</Sheet>

<style>
  .big {
    min-height: 48px;
  }
  form {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 8px;
  }
  input {
    width: 100%;
    min-height: 48px;
    padding: 0 12px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    font-size: 16px;
  }
  input:focus {
    border-color: var(--border-strong);
  }
  .hint {
    margin: 4px 0 0;
    font-size: 13px;
    color: var(--text-faint);
  }
</style>

<script lang="ts">
  import { canShare, copyGif, gifUrl, shareGif, trashGif } from '../lib/actions';
  import type { Gif } from '../lib/api';
  import Sheet from './Sheet.svelte';

  // The long-press menu on a phone (plan 9.3): Copy link, Share, Edit hashtags, Delete.
  let { gif, onedit, onselect, onclose }: { gif: Gif; onedit: (g: Gif) => void; onselect?: (g: Gif) => void; onclose: () => void } = $props();

  // The menu's `gif` prop turns null as soon as the menu closes, so actions take their own copy first.
  // svelte-ignore state_referenced_locally
  const target = gif;
  const url = $derived(gifUrl(gif));
  const title = $derived(gif.tags.length ? gif.tags.map((t) => '#' + t).join(' ') : 'GIF');
</script>

<Sheet label="Actions for {title}" {onclose}>
  {#if url}
    <button
      class="btn row"
      type="button"
      onclick={() => {
        void copyGif(target, 'Link', url);
        onclose();
      }}>Copy link</button
    >
  {/if}
  {#if url && canShare}
    <button
      class="btn row"
      type="button"
      onclick={() => {
        void shareGif(target);
        onclose();
      }}>Share</button
    >
  {/if}
  <button
    class="btn row"
    type="button"
    onclick={() => {
      onclose();
      onedit(target);
    }}>Edit hashtags</button
  >
  {#if onselect}
    <button
      class="btn row"
      type="button"
      onclick={() => {
        onclose();
        onselect(target);
      }}>Select</button
    >
  {/if}
  <button
    class="btn row danger"
    type="button"
    onclick={() => {
      onclose();
      void trashGif(target);
    }}>Delete</button
  >
</Sheet>

<style>
  .row {
    min-height: 48px;
    justify-content: flex-start;
    border-color: transparent;
    font-size: 16px;
  }
</style>

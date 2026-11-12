<script lang="ts">
  import { normalizeTag } from '../lib/hashtags';

  let {
    chips,
    draft = $bindable(''),
    onchips,
    input = $bindable(),
  }: {
    chips: string[];
    draft?: string;
    onchips: (chips: string[]) => void;
    input?: HTMLInputElement;
  } = $props();

  function parse(raw: string): string | null {
    const neg = raw.startsWith('-');
    const t = normalizeTag(neg ? raw.slice(1) : raw);
    return t ? (neg ? `-${t}` : t) : null;
  }

  function commit() {
    const next = [...chips];
    for (const part of draft.split(/[\s,]+/)) {
      const c = part ? parse(part) : null;
      if (c && !next.includes(c)) next.push(c);
    }
    draft = '';
    if (next.length !== chips.length) onchips(next);
  }

  function onInput() {
    draft = draft.replace(/^#+/, '');
    if (/[\s,]$/.test(draft)) commit();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      commit();
    } else if (e.key === 'Backspace' && draft === '' && chips.length) {
      onchips(chips.slice(0, -1));
    } else if (e.key === 'Escape') {
      if (draft) draft = '';
      else input?.blur();
    }
  }
</script>

<div class="search" role="search">
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7" /><path d="m20 20-3.5-3.5" /></svg>
  {#each chips as c (c)}
    <span class="chip tag">
      {c.startsWith('-') ? '−#' + c.slice(1) : '#' + c}
      <button type="button" aria-label="Remove {c}" onclick={() => onchips(chips.filter((x) => x !== c))}>×</button>
    </span>
  {/each}
  <input
    bind:this={input}
    bind:value={draft}
    oninput={onInput}
    onkeydown={onKey}
    class="tag"
    type="search"
    aria-label="Search hashtags"
    placeholder={chips.length ? '' : 'Search hashtags…'}
    autocomplete="off"
    autocapitalize="off"
    spellcheck="false"
  />
  <kbd aria-hidden="true">/</kbd>
</div>

<style>
  .search {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    min-height: 40px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    color: var(--text-muted);
  }
  .search:focus-within {
    border-color: var(--border-strong);
  }
  input {
    flex: 1;
    min-width: 8ch;
    min-height: 30px;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: 16px;
  }
  input::-webkit-search-cancel-button {
    display: none;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-height: 28px;
    padding: 0 10px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 13px;
  }
  .chip button {
    border: 0;
    padding: 0 0 0 2px;
    background: transparent;
    color: inherit;
    font-size: 16px;
    line-height: 1;
  }
  @media (max-width: 639px) {
    kbd {
      display: none;
    }
  }
</style>

<script lang="ts">
  import { api, type Gif, type Tag } from '../lib/api';
  import { app } from '../lib/app.svelte';
  import { MAX_TAGS, cleanDraft, normalizeTag, parseTags } from '../lib/hashtags';
  import Sheet from './Sheet.svelte';

  // Add hashtags to, or remove hashtags from, every selected GIF.
  let {
    mode,
    gifs,
    onsubmit,
    onclose,
  }: {
    mode: 'add' | 'remove';
    /** The selected GIFs that are loaded; "remove" offers the hashtags they carry. */
    gifs: Gif[];
    onsubmit: (tags: string[]) => void;
    onclose: () => void;
  } = $props();

  let chips: string[] = $state([]);
  let draft = $state('');
  let found: Tag[] = $state([]);

  const carried = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const g of gifs) for (const t of g.tags) counts.set(t, (counts.get(t) ?? 0) + 1);
    return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  });
  const recent = $derived(app.recent.slice(0, 12));
  const draftTag = $derived(normalizeTag(draft));

  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  $effect(() => {
    const q = draftTag;
    clearTimeout(timer);
    if (mode !== 'add' || !q) {
      found = [];
      return;
    }
    const mine = ++seq;
    timer = setTimeout(async () => {
      try {
        const page = await api.listTags({ q, sort: 'popular', limit: 6 });
        if (mine === seq) found = page.items.filter((t) => !chips.includes(t.name));
      } catch {
        /* suggestions are optional */
      }
    }, 80);
    return () => clearTimeout(timer);
  });

  function add(...names: string[]) {
    for (const raw of names) {
      const n = normalizeTag(raw);
      if (n && !chips.includes(n) && chips.length < MAX_TAGS) chips.push(n);
    }
  }

  function toggle(name: string) {
    chips = chips.includes(name) ? chips.filter((c) => c !== name) : [...chips, name];
  }

  function onInput() {
    const cleaned = cleanDraft(draft);
    if (cleaned !== draft) draft = cleaned;
    if (/[\s,]/.test(draft)) {
      const ends = /[\s,]$/.test(draft);
      const parts = draft.split(/[\s,]+/).filter(Boolean);
      const rest = ends ? '' : (parts.pop() ?? '');
      add(...parts);
      draft = rest;
    }
  }

  function submit() {
    if (draft.trim()) add(...parseTags(draft));
    draft = '';
    if (chips.length > 0) onsubmit(chips.slice());
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.isComposing) {
      e.preventDefault();
      submit();
    } else if (e.key === 'Backspace' && draft === '' && chips.length > 0) {
      chips.pop();
    }
  }
</script>

<Sheet label={mode === 'add' ? 'Add hashtags to selected GIFs' : 'Remove hashtags from selected GIFs'} {onclose}>
  {#if mode === 'add'}
    <div class="box">
      {#each chips as c (c)}
        <span class="chip tag on">
          #{c}
          <button type="button" aria-label="Remove #{c}" onclick={() => toggle(c)}>×</button>
        </span>
      {/each}
      <input
        bind:value={draft}
        oninput={onInput}
        onkeydown={onKey}
        class="tag"
        type="text"
        aria-label="Hashtags to add"
        placeholder={chips.length ? '' : 'Type a hashtag…'}
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        enterkeyhint="done"
      />
    </div>
    {#if found.length}
      <div class="chips" aria-label="Matching hashtags">
        {#each found as t (t.name)}
          <button type="button" class="chip tag" onclick={() => { add(t.name); draft = ''; }}>#{t.name} <span class="n">{t.gif_count}</span></button>
        {/each}
      </div>
    {:else if recent.length}
      <p class="label">Recent</p>
      <div class="chips">
        {#each recent as name (name)}
          <button type="button" class="chip tag" class:on={chips.includes(name)} aria-pressed={chips.includes(name)} onclick={() => toggle(name)}>#{name}</button>
        {/each}
      </div>
    {/if}
  {:else if carried.length}
    <p class="label">Tap the hashtags to take off</p>
    <div class="chips">
      {#each carried as [name, n] (name)}
        <button type="button" class="chip tag" class:on={chips.includes(name)} aria-pressed={chips.includes(name)} onclick={() => toggle(name)}>
          #{name} <span class="n">{n}</span>
        </button>
      {/each}
    </div>
  {:else}
    <p class="label">The selected GIFs have no hashtags.</p>
  {/if}
  <div class="foot">
    <button class="btn" type="button" onclick={onclose}>Cancel</button>
    <button class="btn primary" type="button" disabled={chips.length === 0 && !draftTag} onclick={submit}>
      {mode === 'add' ? 'Add' : 'Remove'}
    </button>
  </div>
</Sheet>

<style>
  .box {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    min-height: 44px;
    padding: 6px 8px;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
  }
  input {
    flex: 1;
    min-width: 120px;
    min-height: 32px;
    border: 0;
    background: transparent;
    outline: none;
    font-size: 16px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 32px;
    padding: 0 10px;
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    background: transparent;
    font-size: 13px;
  }
  .chip.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .chip button {
    border: 0;
    background: transparent;
    padding: 0 2px;
    color: inherit;
  }
  .n {
    color: var(--text-faint);
    font-size: 12px;
  }
  .chip.on .n {
    color: inherit;
    opacity: 0.6;
  }
  .label {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--text-muted);
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 8px;
  }
</style>

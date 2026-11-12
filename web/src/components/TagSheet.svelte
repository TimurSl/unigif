<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, mediaPath, type Gif, type Tag } from '../lib/api';
  import { app } from '../lib/app.svelte';
  import { swipe } from '../lib/gestures';
  import { MAX_TAGS, cleanDraft, normalizeTag, parseTags } from '../lib/hashtags';

  let {
    gif,
    mode,
    heading,
    proceed = $bindable(true),
    canGoBack = false,
    onsave,
    onskip,
    onprev,
    ontrash,
    onclose,
  }: {
    gif: Gif;
    mode: 'queue' | 'edit';
    heading: string;
    proceed?: boolean;
    canGoBack?: boolean;
    /** `opposite` is true for Shift+Enter: do the reverse of the "proceed" checkbox, once. */
    onsave: (tags: string[], opposite: boolean) => void;
    onskip: () => void;
    onprev: () => void;
    ontrash: () => void;
    onclose: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let chips: string[] = $state(gif.tags.slice());
  let draft = $state('');
  let suggestions: Tag[] = $state([]);
  let active = $state(-1);
  let open = $state(false);
  let shake = $state(false);
  let hint = $state('');
  let input: HTMLInputElement | undefined = $state();
  let dialog: HTMLDivElement | undefined = $state();
  let loaded = $state(false);

  const draftTag = $derived(normalizeTag(draft));
  const canCreate = $derived(draftTag !== null && !chips.includes(draftTag) && !suggestions.some((s) => s.name === draftTag));
  const rows = $derived([
    ...suggestions.map((s) => ({ name: s.name, count: s.gif_count, create: false })),
    ...(canCreate && draftTag ? [{ name: draftTag, count: 0, create: true }] : []),
  ]);
  const recent = $derived(app.recent.slice(0, 12));
  const hasLast = $derived(app.lastTags.length > 0);
  const src = $derived(gif.media?.mp4 ? mediaPath(gif.media.mp4.url) : null);
  const poster = $derived(gif.media?.poster ? mediaPath(gif.media.poster.url) : null);

  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  $effect(() => {
    const q = draftTag;
    clearTimeout(timer);
    if (!q) {
      suggestions = [];
      active = -1;
      return;
    }
    const mine = ++seq;
    timer = setTimeout(async () => {
      try {
        const page = await api.listTags({ q, sort: 'popular', limit: 8 });
        if (mine === seq) {
          suggestions = page.items.filter((t) => !chips.includes(t.name));
          active = -1;
        }
      } catch {
        /* suggestions are optional */
      }
    }, 60);
  });

  onMount(() => {
    loaded = true;
    input?.focus();
    return () => clearTimeout(timer);
  });

  function add(...names: string[]) {
    for (const raw of names) {
      const n = normalizeTag(raw);
      if (!n || chips.includes(n)) continue;
      if (chips.length >= MAX_TAGS) {
        flash(`At most ${MAX_TAGS} hashtags per GIF`);
        break;
      }
      chips.push(n);
    }
    hint = '';
  }

  function toggle(name: string) {
    if (chips.includes(name)) chips = chips.filter((c) => c !== name);
    else add(name);
    input?.focus();
  }

  function commitDraft() {
    if (draft.trim()) add(...parseTags(draft));
    draft = '';
    open = false;
  }

  function flash(msg: string) {
    hint = msg;
    shake = false;
    void tick().then(() => {
      shake = true;
      setTimeout(() => (shake = false), 400);
    });
  }

  function submit(opposite = false) {
    commitDraft();
    if (chips.length === 0) {
      flash('Add a hashtag, or Skip');
      input?.focus();
      return;
    }
    onsave(chips.slice(), opposite);
  }

  function accept(row: { name: string }) {
    add(row.name);
    draft = '';
    open = false;
    active = -1;
    input?.focus();
  }

  function onInput() {
    const cleaned = cleanDraft(draft);
    if (cleaned !== draft) draft = cleaned;
    // Space or comma ends a tag. Done on input (not keydown) so mobile keyboards work too.
    if (/[\s,]/.test(draft)) {
      const endsWithSeparator = /[\s,]$/.test(draft);
      const parts = draft.split(/[\s,]+/).filter(Boolean);
      const rest = endsWithSeparator ? '' : (parts.pop() ?? '');
      add(...parts);
      draft = rest;
    }
    open = draft.length > 0;
  }

  function onPaste(e: ClipboardEvent) {
    const text = e.clipboardData?.getData('text') ?? '';
    if (!text) return;
    e.preventDefault();
    add(...parseTags(text));
  }

  function onKey(e: KeyboardEvent) {
    // Alt + digit toggles a recent hashtag; Alt+0 is "same as last". `code` because Alt changes `key` on macOS.
    if (e.altKey && !e.ctrlKey && !e.metaKey) {
      const m = e.code.match(/^Digit(\d)$/);
      if (m) {
        e.preventDefault();
        const n = Number(m[1]);
        if (n === 0) sameAsLast();
        else if (recent[n - 1]) toggle(recent[n - 1]);
        return;
      }
      if (mode === 'queue' && e.key === 'ArrowRight') {
        e.preventDefault();
        onskip();
        return;
      }
      if (mode === 'queue' && e.key === 'ArrowLeft') {
        e.preventDefault();
        if (canGoBack) onprev();
        return;
      }
    }
    if (e.key === 'Delete' && e.shiftKey) {
      e.preventDefault();
      ontrash();
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (open && rows.length > 0) open = false;
      else onclose();
      return;
    }
    if (e.target !== input) return;

    if (e.key === 'Enter') {
      e.preventDefault();
      if (e.isComposing) return;
      if (active >= 0 && rows[active]) accept(rows[active]);
      else submit(e.shiftKey);
    } else if (e.key === 'Tab' && !e.shiftKey && open && rows.length > 0) {
      e.preventDefault();
      accept(rows[Math.max(active, 0)]);
    } else if (e.key === 'ArrowDown' && rows.length > 0) {
      e.preventDefault();
      open = true;
      active = (active + 1) % rows.length;
    } else if (e.key === 'ArrowUp' && rows.length > 0) {
      e.preventDefault();
      open = true;
      active = active <= 0 ? rows.length - 1 : active - 1;
    } else if (e.key === 'Backspace' && draft === '' && chips.length > 0) {
      chips.pop();
    }
  }

  function sameAsLast() {
    add(...app.lastTags);
    input?.focus();
  }

  // Keep focus inside the sheet while it is open.
  function onFocusOut(e: FocusEvent) {
    const to = e.relatedTarget as Node | null;
    if (to && dialog && !dialog.contains(to)) input?.focus();
  }
</script>

<!-- On the window, so the shortcuts keep working if focus ends up on the backdrop. -->
<svelte:window onkeydown={onKey} />

<div
  class="sheet"
  role="dialog"
  aria-modal="true"
  aria-label={heading}
  tabindex="-1"
  bind:this={dialog}
  onfocusout={onFocusOut}
>
  <header>
    <h2>{heading}</h2>
    <button class="btn ghost" type="button" aria-label="Close" onclick={onclose}>
      <kbd>Esc</kbd> ✕
    </button>
  </header>

  <div class="body">
    <div class="stage">
      <div
        class="player"
        style:aspect-ratio={gif.width && gif.height ? `${gif.width} / ${gif.height}` : undefined}
        use:swipe={{
          left: mode === 'queue' ? onskip : undefined,
          right: mode === 'queue' && canGoBack ? onprev : undefined,
          down: onclose,
        }}
      >
        {#if src}
          <video {src} poster={poster ?? undefined} autoplay muted loop playsinline controlslist="nodownload"></video>
        {:else}
          <p class="note">{gif.status === 'failed' ? 'Could not be processed' : 'Processing…'}</p>
        {/if}
      </div>
      {#if mode === 'queue'}
        <div class="nav">
          <button class="btn ghost" type="button" disabled={!canGoBack} onclick={onprev}>← Prev <kbd>Alt ←</kbd></button>
          <button class="btn ghost" type="button" onclick={onskip}>Skip → <kbd>Alt →</kbd></button>
          <button class="btn ghost danger" type="button" onclick={ontrash}>Delete <kbd>⇧ Del</kbd></button>
        </div>
      {:else}
        <div class="nav">
          <button class="btn ghost danger" type="button" onclick={ontrash}>Delete <kbd>⇧ Del</kbd></button>
        </div>
      {/if}
    </div>

    <div class="tagging">
      <section aria-label="Recent hashtags">
        <h3>Recent</h3>
        <div class="recent">
          {#each recent as name, i (name)}
            <button
              type="button"
              class="chip tag"
              class:on={chips.includes(name)}
              aria-pressed={chips.includes(name)}
              title={i < 9 ? `Alt+${i + 1}` : undefined}
              onclick={() => toggle(name)}
            >#{name}</button>
          {:else}
            {#if loaded}<span class="muted">Hashtags you use show up here.</span>{/if}
          {/each}
        </div>
        {#if hasLast}
          <button type="button" class="same" onclick={sameAsLast}>
            ↻ Same as last: <span class="tag">{app.lastTags.map((t) => '#' + t).join(' ')}</span> <kbd>Alt 0</kbd>
          </button>
        {/if}
      </section>

      <div class="field" class:shake>
        <div class="box">
          {#each chips as c (c)}
            <span class="chip tag on">
              #{c}
              <button type="button" aria-label="Remove #{c}" onclick={() => (chips = chips.filter((x) => x !== c))}>×</button>
            </span>
          {/each}
          <input
            bind:this={input}
            bind:value={draft}
            oninput={onInput}
            onpaste={onPaste}
            onfocus={() => (open = draft.length > 0)}
            class="tag"
            type="text"
            role="combobox"
            aria-label="Hashtags"
            aria-expanded={open && rows.length > 0}
            aria-controls="tag-suggestions"
            aria-activedescendant={active >= 0 ? `sug-${active}` : undefined}
            aria-autocomplete="list"
            placeholder={chips.length ? '' : 'Type a hashtag…'}
            autocomplete="off"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            enterkeyhint="done"
          />
        </div>
        {#if open && rows.length > 0}
          <ul id="tag-suggestions" role="listbox" aria-label="Hashtag suggestions">
            {#each rows as row, i (row.name + row.create)}
              <li
                id="sug-{i}"
                role="option"
                aria-selected={i === active}
                class:active={i === active}
              >
                <button type="button" tabindex="-1" onmousedown={(e) => e.preventDefault()} onclick={() => accept(row)}>
                  {#if row.create}
                    <span class="tag">+ create #{row.name}</span>
                  {:else}
                    <span class="tag">#{row.name}</span><span class="count">{row.count}</span>
                  {/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
        <p class="hint" role="status">{hint}</p>
      </div>

      <footer>
        {#if mode === 'queue'}
          <label class="check">
            <input type="checkbox" bind:checked={proceed} />
            Proceed to next unsorted
          </label>
        {:else}
          <span></span>
        {/if}
        <button class="btn primary" type="button" onclick={() => submit(false)}>Save <kbd>⏎</kbd></button>
      </footer>
    </div>
  </div>
</div>

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    width: min(1040px, 100%);
    max-height: min(720px, 100%);
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-tile);
    outline: none;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 8px 8px 16px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-muted);
  }
  h3 {
    margin: 0 0 8px;
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .body {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(300px, 1fr);
    min-height: 0;
    flex: 1;
  }
  .stage {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--border);
  }
  .player {
    flex: 1;
    min-height: 200px;
    touch-action: none; /* swipes are ours: left skips, right goes back, down closes */
    display: grid;
    place-items: center;
    padding: 16px;
    background: #000;
    overflow: hidden;
  }
  .player video {
    max-width: 100%;
    max-height: 100%;
    border-radius: 6px;
  }
  .note {
    color: var(--text-muted);
  }
  .nav {
    display: flex;
    justify-content: space-between;
    gap: 4px;
    padding: 8px;
    border-top: 1px solid var(--border);
  }
  .tagging {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px;
    min-width: 0;
    overflow-y: auto;
  }
  .recent {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .muted {
    color: var(--text-faint);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-height: 32px;
    padding: 0 10px;
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    background: transparent;
    font-size: 13px;
    transition: background var(--motion);
  }
  button.chip:hover {
    background: var(--surface-2);
  }
  .chip.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  button.chip.on:hover {
    background: #e6e6e6;
  }
  .chip button {
    border: 0;
    background: transparent;
    padding: 0 0 0 2px;
    color: inherit;
    font-size: 16px;
    line-height: 1;
  }
  .same {
    margin-top: 10px;
    padding: 6px 0;
    border: 0;
    background: transparent;
    color: var(--text-muted);
    text-align: left;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .same:hover {
    color: var(--text);
  }
  .field {
    position: relative;
  }
  .box {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    padding: 6px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
  }
  .box:focus-within {
    border-color: var(--border-strong);
  }
  .box input {
    flex: 1;
    min-width: 8ch;
    min-height: 32px;
    border: 0;
    outline: none;
    background: transparent;
    font-size: 16px;
  }
  ul {
    position: absolute;
    z-index: 2;
    left: 0;
    right: 0;
    margin: 4px 0 0;
    padding: 4px;
    list-style: none;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
  }
  li button {
    display: flex;
    justify-content: space-between;
    width: 100%;
    min-height: 36px;
    align-items: center;
    padding: 0 8px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    text-align: left;
  }
  li.active button,
  li button:hover {
    background: var(--border);
  }
  .count {
    color: var(--text-muted);
  }
  .hint {
    min-height: 20px;
    margin: 6px 2px 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  .shake .box {
    animation: shake 300ms;
    border-color: var(--text-muted);
  }
  @keyframes shake {
    20%, 60% { transform: translateX(-4px); }
    40%, 80% { transform: translateX(4px); }
  }
  footer {
    margin-top: auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-muted);
  }
  .check input {
    width: 18px;
    height: 18px;
    accent-color: #fff;
  }
  .btn kbd {
    border-color: currentColor;
    opacity: 0.6;
  }
  /* Phone (plan 9.5): full screen, GIF at most 40 % of the height, the input and Save pinned at the
     bottom (the sheet itself ends above the keyboard; see lib/viewport.ts). */
  @media (max-width: 720px) {
    .sheet {
      height: 100%;
      max-height: none;
      border: 0;
      border-radius: 0;
    }
    header {
      padding-top: calc(4px + env(safe-area-inset-top));
    }
    header .btn {
      min-width: 44px;
      min-height: 44px;
    }
    .body {
      display: flex;
      flex-direction: column;
    }
    .stage {
      flex: 0 1 auto;
      max-height: 40%;
      border-right: 0;
      border-bottom: 1px solid var(--border);
    }
    .player {
      min-height: 96px;
      padding: 8px;
    }
    .nav {
      padding: 4px;
    }
    .nav .btn {
      min-height: 44px;
      padding: 0 10px;
    }
    .nav kbd,
    header kbd,
    footer kbd,
    .same kbd {
      display: none;
    }
    .tagging {
      flex: 1 1 0;
      gap: 12px;
      padding: 12px 12px 0;
    }
    h3 {
      display: none; /* the chips speak for themselves, and the height is precious */
    }
    .recent {
      flex-wrap: nowrap;
      overflow-x: auto;
      margin: 0 -12px;
      padding: 0 12px;
      scrollbar-width: none;
    }
    .recent .chip {
      flex: none;
    }
    .chip {
      min-height: 40px;
      padding: 0 14px;
    }
    .chip button {
      min-width: 32px;
      min-height: 32px;
      margin-right: -6px;
    }
    .same {
      margin-top: 4px;
      min-height: 44px;
    }
    .box input {
      min-height: 40px;
    }
    /* Suggestions flow under the input instead of floating over the pinned Save row. */
    .field ul {
      position: static;
      margin-top: 6px;
    }
    li button {
      min-height: 44px;
    }
    footer {
      position: sticky;
      bottom: 0;
      margin: auto -12px 0;
      padding: 8px 12px calc(8px + env(safe-area-inset-bottom));
      background: var(--surface-1);
      border-top: 1px solid var(--border);
    }
    footer .btn {
      min-height: 44px;
      min-width: 96px;
    }
    .check {
      min-height: 44px;
    }
    .check input {
      width: 22px;
      height: 22px;
    }
  }
</style>

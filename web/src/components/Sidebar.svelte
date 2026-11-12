<script lang="ts">
  import { app, logout } from '../lib/app.svelte';
  import { href, linkClick, route } from '../lib/router.svelte';

  let { onadd }: { onadd: () => void } = $props();
</script>

<!-- Desktop: sidebar. Tablet: icon rail. Phone: bottom tab bar (Library, Unsorted, Add, Settings). -->
<nav class="side" aria-label="Main">
  <a class="brand" href="/" onclick={(e) => linkClick(e, '/')}>unigif</a>
  <ul class="main">
    <li>
      <a class="tab" href="/" class:current={route.view === 'library' && !route.q} onclick={(e) => linkClick(e, '/')}>
        <svg class="ico" viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="7" height="9" rx="1.5" /><rect x="14" y="3" width="7" height="5" rx="1.5" /><rect x="14" y="12" width="7" height="9" rx="1.5" /><rect x="3" y="16" width="7" height="5" rx="1.5" /></svg>
        <span>Library</span>
      </a>
    </li>
    <li>
      <a class="tab" href="/unsorted" class:current={route.view === 'unsorted'} onclick={(e) => linkClick(e, '/unsorted')}>
        <svg class="ico" viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 13h5l1.5 3h5L16 13h5" /><path d="M5.5 5h13L21 13v6a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-6z" /></svg>
        <span>Unsorted</span>
        {#if app.stats && app.stats.unsorted > 0}<span class="badge" data-testid="unsorted-count">{app.stats.unsorted}</span>{/if}
      </a>
    </li>
    <li>
      <a class="tab" href="/search" class:current={route.view === 'search'} onclick={(e) => linkClick(e, '/search')}>
        <svg class="ico" viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7" /><path d="m20 20-3.5-3.5" /></svg>
        <span>Search</span>
      </a>
    </li>
    <li class="phone-only">
      <button class="tab" type="button" onclick={onadd}>
        <svg class="ico" viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
        <span>Add</span>
      </button>
    </li>
  </ul>
  {#if app.topTags.length}
    <h2>
      Hashtags
      <a class="manage" href="/tags" onclick={(e) => linkClick(e, '/tags')}>Manage</a>
    </h2>
    <ul class="tags">
      {#each app.topTags as t (t.name)}
        <li>
          <a
            class="tab tag"
            href={href('library', t.name)}
            class:current={route.view === 'library' && route.q === t.name}
            onclick={(e) => linkClick(e, href('library', t.name))}
          >
            <span>#{t.name}</span><span class="count">{t.gif_count}</span>
          </a>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="foot">
    <a class="tab trash-tab" href="/trash" class:current={route.view === 'trash'} onclick={(e) => linkClick(e, '/trash')}>
      <svg class="ico" viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-12M9 7V4h6v3" /></svg>
      <span>Trash</span>
      {#if app.stats && app.stats.trash > 0}<span class="count">{app.stats.trash}</span>{/if}
    </a>
    <a class="tab" href="/settings" class:current={route.view === 'settings'} onclick={(e) => linkClick(e, '/settings')}>
      <svg class="ico" viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M4 7h10M18 7h2M4 17h2M10 17h10" /><circle cx="16" cy="7" r="2" /><circle cx="8" cy="17" r="2" /></svg>
      <span>Settings</span>
    </a>
    <div class="account">
      <span class="who">{app.me?.name}</span>
      <button class="btn ghost" type="button" onclick={logout}>Sign out</button>
    </div>
  </div>
</nav>

<style>
  .side {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 220px;
    flex: none;
    height: 100dvh;
    padding: 12px 8px;
    background: var(--surface-1);
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }
  .brand {
    padding: 4px 12px 12px;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--text);
    text-decoration: none;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .tab {
    display: flex;
    justify-content: space-between;
    align-items: center;
    width: 100%;
    min-height: 36px;
    padding: 0 12px;
    border: 0;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--text-muted);
    text-decoration: none;
  }
  .tab:hover,
  .tab.current {
    background: var(--surface-2);
    color: var(--text);
  }
  .ico,
  .phone-only {
    display: none;
  }
  h2 {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin: 16px 12px 4px;
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .manage {
    font-size: 12px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--text-muted);
  }
  .tags .tab {
    min-height: 32px;
    font-size: 13px;
  }
  .count,
  .badge {
    font-size: 12px;
    color: var(--text-muted);
  }
  .badge {
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: 999px;
  }
  .foot {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .account {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 4px 0 12px;
    border-top: 1px solid var(--border);
  }
  .who {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-muted);
  }

  /* Tablet: an icon rail. */
  @media (max-width: 1023px) and (min-width: 640px) {
    .side {
      width: 72px;
    }
    .brand,
    h2,
    .tags,
    .account,
    .badge,
    .foot .count {
      display: none;
    }
    .ico {
      display: block;
    }
    .tab {
      flex-direction: column;
      justify-content: center;
      gap: 2px;
      min-height: 56px;
      padding: 0;
      font-size: 11px;
    }
  }

  /* Phone: a bottom tab bar. */
  @media (max-width: 639px) {
    .side {
      position: fixed;
      z-index: 20;
      bottom: 0;
      left: 0;
      right: 0;
      width: auto;
      height: var(--tabbar-h);
      flex-direction: row;
      gap: 0;
      padding: 0 4px env(safe-area-inset-bottom);
      border-right: 0;
      border-top: 1px solid var(--border);
      overflow: visible;
    }
    .brand,
    h2,
    .tags,
    .account {
      display: none;
    }
    .main {
      display: flex;
      flex: 4;
    }
    .main li {
      flex: 1;
    }
    .foot {
      margin: 0;
      flex: 1;
    }
    .trash-tab {
      display: none;
    }
    .phone-only {
      display: block;
    }
    .ico {
      display: block;
    }
    .tab {
      position: relative;
      flex-direction: column;
      justify-content: center;
      gap: 2px;
      height: 100%;
      min-height: 48px;
      padding: 0;
      font-size: 11px;
    }
    .badge {
      position: absolute;
      top: 4px;
      left: calc(50% + 6px);
      padding: 0 6px;
      background: var(--accent);
      border-color: var(--accent);
      color: var(--on-accent);
      font-size: 11px;
      line-height: 16px;
    }
  }
</style>

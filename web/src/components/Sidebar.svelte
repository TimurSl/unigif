<script lang="ts">
  import { app, logout } from '../lib/app.svelte';
  import { href, navigate, route } from '../lib/router.svelte';

  function go(e: MouseEvent, to: string) {
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
    e.preventDefault();
    navigate(to);
  }
</script>

<nav class="side" aria-label="Main">
  <a class="brand" href="/" onclick={(e) => go(e, '/')}>unigif</a>
  <ul class="main">
    <li>
      <a href="/" class:current={route.view === 'library' && !route.q} onclick={(e) => go(e, '/')}>Library</a>
    </li>
    <li>
      <a href="/unsorted" class:current={route.view === 'unsorted'} onclick={(e) => go(e, '/unsorted')}>
        Unsorted
        {#if app.stats && app.stats.unsorted > 0}<span class="badge" data-testid="unsorted-count">{app.stats.unsorted}</span>{/if}
      </a>
    </li>
  </ul>
  {#if app.topTags.length}
    <h2>Hashtags</h2>
    <ul class="tags">
      {#each app.topTags as t (t.name)}
        <li>
          <a
            class="tag"
            href={href('library', t.name)}
            class:current={route.view === 'library' && route.q === t.name}
            onclick={(e) => go(e, href('library', t.name))}
          >
            <span>#{t.name}</span><span class="count">{t.gif_count}</span>
          </a>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="foot">
    <span class="who">{app.me?.name}</span>
    <button class="btn ghost" type="button" onclick={logout}>Sign out</button>
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
  a {
    display: flex;
    justify-content: space-between;
    align-items: center;
    min-height: 36px;
    padding: 0 12px;
    border-radius: var(--radius-control);
    color: var(--text-muted);
    text-decoration: none;
  }
  a:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  a.current {
    background: var(--surface-2);
    color: var(--text);
  }
  h2 {
    margin: 16px 12px 4px;
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .tags a {
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
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 12px 4px 0 12px;
    border-top: 1px solid var(--border);
  }
  .who {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-muted);
  }

  @media (max-width: 1023px) and (min-width: 640px) {
    .side {
      width: 64px;
    }
    .brand,
    h2,
    .tags,
    .who,
    .badge {
      display: none;
    }
    .foot {
      padding: 8px 0 0;
      justify-content: center;
    }
    a {
      justify-content: center;
      padding: 0;
      font-size: 12px;
    }
  }

  /* Phone: a bottom tab bar (the fuller mobile pass is milestone 4). */
  @media (max-width: 639px) {
    .side {
      position: fixed;
      z-index: 20;
      bottom: 0;
      left: 0;
      right: 0;
      width: auto;
      height: auto;
      flex-direction: row;
      padding: 4px 8px calc(4px + env(safe-area-inset-bottom));
      border-right: 0;
      border-top: 1px solid var(--border);
      overflow: visible;
    }
    .brand,
    h2,
    .tags,
    .who {
      display: none;
    }
    .main {
      display: flex;
      flex: 1;
      gap: 4px;
    }
    .main li {
      flex: 1;
    }
    .main a {
      justify-content: center;
      gap: 6px;
      min-height: 44px;
    }
    .foot {
      margin: 0;
      padding: 0;
      border: 0;
    }
  }
</style>

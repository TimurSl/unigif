// A very small path router. Routes: / (library), /unsorted, /search, /settings, /login. A GIF's detail view is an overlay
// on top of the current list, addressed by `?gif=<id>`, so the list underneath keeps its place.

export interface Route {
  view: 'library' | 'unsorted' | 'search' | 'settings' | 'login';
  q: string;
  /** Search view only: the provider being searched. */
  provider: string;
  gif: string | null;
}

function parse(): Route {
  const params = new URLSearchParams(location.search);
  const path = location.pathname;
  const view = path.startsWith('/unsorted')
    ? 'unsorted'
    : path.startsWith('/search')
      ? 'search'
      : path.startsWith('/settings')
      ? 'settings'
      : path.startsWith('/login')
        ? 'login'
        : 'library';
  return { view, q: params.get('q') ?? '', provider: params.get('provider') ?? '', gif: params.get('gif') };
}

export const route: Route = $state(parse());

function sync() {
  Object.assign(route, parse());
}
window.addEventListener('popstate', sync);

export function href(view: Route['view'], q = ''): string {
  const base = view === 'unsorted' ? '/unsorted' : view === 'search' ? '/search' : view === 'settings' ? '/settings' : view === 'login' ? '/login' : '/';
  return q ? `${base}?q=${encodeURIComponent(q)}` : base;
}

export function navigate(to: string, opts: { replace?: boolean } = {}) {
  if (opts.replace) history.replaceState(null, '', to);
  else history.pushState(null, '', to);
  sync();
}

/** Click handler for in-app links: plain clicks navigate without a reload, modified clicks keep the browser's behaviour. */
export function linkClick(e: MouseEvent, to: string) {
  if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
  e.preventDefault();
  navigate(to);
}

function withGif(id: string | null): string {
  const params = new URLSearchParams(location.search);
  if (id) params.set('gif', id);
  else params.delete('gif');
  const s = params.toString();
  return location.pathname + (s ? `?${s}` : '');
}

export function openGif(id: string) {
  history.pushState({ overlay: true }, '', withGif(id));
  sync();
}

export function closeGif() {
  if ((history.state as { overlay?: boolean } | null)?.overlay) history.back();
  else navigate(withGif(null), { replace: true });
}

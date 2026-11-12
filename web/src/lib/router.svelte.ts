// A very small path router. Routes: / (library), /unsorted, /login. A GIF's detail view is an overlay
// on top of the current list, addressed by `?gif=<id>`, so the list underneath keeps its place.

export interface Route {
  view: 'library' | 'unsorted' | 'login';
  q: string;
  gif: string | null;
}

function parse(): Route {
  const params = new URLSearchParams(location.search);
  const path = location.pathname;
  const view = path.startsWith('/unsorted') ? 'unsorted' : path.startsWith('/login') ? 'login' : 'library';
  return { view, q: params.get('q') ?? '', gif: params.get('gif') };
}

export const route: Route = $state(parse());

function sync() {
  Object.assign(route, parse());
}
window.addEventListener('popstate', sync);

export function href(view: Route['view'], q = ''): string {
  const base = view === 'unsorted' ? '/unsorted' : view === 'login' ? '/login' : '/';
  return q ? `${base}?q=${encodeURIComponent(q)}` : base;
}

export function navigate(to: string, opts: { replace?: boolean } = {}) {
  if (opts.replace) history.replaceState(null, '', to);
  else history.pushState(null, '', to);
  sync();
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

import { api, ApiError, setUnauthorizedHandler, type Me, type Stats, type Tag, type Gif } from './api';
import { navigate, route } from './router.svelte';

export interface Toast {
  id: number;
  text: string;
  kind: 'info' | 'error';
  action?: { label: string; run: () => void };
}

export interface Prefs {
  proceed_to_next: boolean;
  unsorted_sort: 'new' | 'old';
}

const PREF_KEY = 'unigif.prefs';

function loadLocalPrefs(): Prefs {
  const base: Prefs = { proceed_to_next: true, unsorted_sort: 'new' };
  try {
    return { ...base, ...JSON.parse(localStorage.getItem(PREF_KEY) ?? '{}') };
  } catch {
    return base;
  }
}

export const app = $state({
  phase: 'loading' as 'loading' | 'anon' | 'ready',
  me: null as Me | null,
  stats: null as Stats | null,
  topTags: [] as Tag[],
  prefs: loadLocalPrefs(),
  /** Hashtags of the GIF saved last, for "Same as last". */
  lastTags: [] as string[],
  /** Most recently used hashtags, newest first; session picks move to the front. */
  recent: [] as string[],
  toasts: [] as Toast[],
  upload: null as { done: number; total: number } | null,
});

let toastId = 0;

export function toast(text: string, opts: { kind?: 'info' | 'error'; action?: Toast['action']; ms?: number } = {}) {
  const t: Toast = { id: ++toastId, text, kind: opts.kind ?? 'info', action: opts.action };
  app.toasts.push(t);
  setTimeout(() => dismissToast(t.id), opts.ms ?? (opts.action ? 6000 : 3500));
  return t.id;
}

export function dismissToast(id: number) {
  const i = app.toasts.findIndex((t) => t.id === id);
  if (i >= 0) app.toasts.splice(i, 1);
}

export function errorText(e: unknown): string {
  return e instanceof ApiError ? e.detail || e.code : e instanceof Error ? e.message : String(e);
}

setUnauthorizedHandler(() => {
  if (app.phase === 'ready') {
    app.phase = 'anon';
    app.me = null;
    navigate('/login', { replace: true });
  }
});

export async function boot() {
  try {
    app.me = await api.me();
    app.phase = 'ready';
    const server = app.me.prefs as Partial<Prefs> | undefined;
    if (server) {
      if (typeof server.proceed_to_next === 'boolean') app.prefs.proceed_to_next = server.proceed_to_next;
      if (server.unsorted_sort === 'new' || server.unsorted_sort === 'old') app.prefs.unsorted_sort = server.unsorted_sort;
    }
    if (route.view === 'login') navigate('/', { replace: true });
    void refreshStats();
    void refreshTopTags();
  } catch (e) {
    app.phase = 'anon';
    if (e instanceof ApiError && e.status !== 401) toast(errorText(e), { kind: 'error' });
    if (route.view !== 'login') navigate('/login', { replace: true });
  }
}

export async function login(username: string, password: string) {
  await api.login(username, password);
  app.phase = 'loading';
  await boot();
}

export async function logout() {
  try {
    await api.logout();
  } finally {
    app.phase = 'anon';
    app.me = null;
    navigate('/login', { replace: true });
  }
}

export async function refreshStats() {
  try {
    app.stats = await api.stats();
  } catch {
    /* the badge just stays as it was */
  }
}

let tagsTimer: ReturnType<typeof setTimeout> | undefined;
export function refreshTopTags(soon = false) {
  clearTimeout(tagsTimer);
  const run = async () => {
    try {
      app.topTags = (await api.listTags({ sort: 'popular', limit: 100 })).items;
    } catch {
      /* keep the previous list */
    }
  };
  if (soon) tagsTimer = setTimeout(run, 400);
  else void run();
}

export function setPref<K extends keyof Prefs>(key: K, value: Prefs[K]) {
  app.prefs[key] = value;
  try {
    localStorage.setItem(PREF_KEY, JSON.stringify(app.prefs));
  } catch {
    /* private mode */
  }
  api.setPrefs({ ...app.prefs }).catch(() => {});
}

export async function loadRecent() {
  try {
    const s = await api.suggest([], 12);
    const mine = app.recent.filter((t) => !s.recent.some((r) => r.name === t));
    app.recent = [...mine, ...s.recent.map((t) => t.name)].slice(0, 12);
  } catch {
    /* keep what we have */
  }
}

/** Called after a GIF is saved with hashtags. */
export function noteSaved(tags: string[]) {
  app.lastTags = tags.slice();
  app.recent = [...tags, ...app.recent.filter((t) => !tags.includes(t))].slice(0, 12);
  refreshTopTags(true);
}

// ---- GIF change notifications between the sheet, grids and detail view ----

type GifListener = (kind: 'changed' | 'removed' | 'added', gif: Gif) => void;
const listeners = new Set<GifListener>();

export function onGifEvent(fn: GifListener): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function emitGif(kind: 'changed' | 'removed' | 'added', gif: Gif) {
  for (const l of listeners) l(kind, gif);
}

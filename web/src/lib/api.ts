// Typed client for /api/v1. Cookie session, same origin.

export type Scope = 'read' | 'upload' | 'tag' | 'delete' | 'admin';

export interface MediaFile {
  url: string;
  width: number | null;
  height: number | null;
  bytes: number;
}

export interface OnDemandFile {
  url: string;
  on_demand: true;
}

export interface Gif {
  id: string;
  status: 'processing' | 'ready' | 'failed';
  tags: string[];
  description: string | null;
  width: number | null;
  height: number | null;
  duration_ms: number | null;
  has_alpha: boolean;
  thumbhash: string | null;
  source: { type: string; id: string | null };
  created_at: string;
  sorted_at: string | null;
  /** Public share page (OpenGraph tags, so a pasted link unfurls). */
  page_url: string;
  media: {
    mp4: MediaFile | null;
    preview_mp4: MediaFile | null;
    poster: MediaFile | null;
    // Converted the first time the URL is fetched.
    gif: OnDemandFile;
    preview_gif: OnDemandFile;
    webm: OnDemandFile;
    webp: OnDemandFile;
  } | null;
  duplicate?: boolean;
}

export interface Tag {
  name: string;
  gif_count: number;
  last_used_at: string | null;
}

export interface Me {
  kind: 'user' | 'key';
  name: string;
  scopes: Scope[];
  deny_tags: string[];
  prefs?: Record<string, unknown>;
}

export interface Stats {
  gifs: number;
  unsorted: number;
  tags: number;
  trash: number;
  bytes_stored: number;
  bytes_original: number;
  bytes_saved: number;
  cache_files: number;
  cache_bytes: number;
  cache_limit: number;
}

export type ProviderId = 'klipy' | 'giphy';

export interface SearchProvider {
  id: ProviderId;
  name: string;
  configured: boolean;
  key_source: 'settings' | 'config' | null;
  /** Where to get an API key. */
  keys_url: string;
}

export interface SearchItem {
  id: string;
  title: string;
  width: number | null;
  height: number | null;
  page_url: string | null;
  preview: { url: string; kind: 'video' | 'image' };
  download: { url: string; format: 'gif' | 'mp4'; bytes: number | null };
}

export interface ApiKey {
  id: string;
  name: string;
  scopes: Scope[];
  deny_tags: string[];
  rate_limit: number | null;
  created_at: string;
  expires_at: string | null;
  last_used_at: string | null;
  revoked_at: string | null;
}

export type KeyPreset = 'read_only' | 'uploader' | 'picker' | 'full';

export interface Page<T> {
  items: T[];
  next_cursor: string | null;
}

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    public detail: string,
  ) {
    super(detail || code);
  }
}

let onUnauthorized: () => void = () => {};
export function setUnauthorizedHandler(fn: () => void) {
  onUnauthorized = fn;
}

async function request<T>(method: string, path: string, init: { body?: unknown; raw?: BodyInit; headers?: Record<string, string> } = {}): Promise<T> {
  // Cookie-authenticated writes need JSON or this header (the server's CSRF rule); send it on every call.
  const headers: Record<string, string> = { 'X-Unigif-Request': '1', ...init.headers };
  let body: BodyInit | undefined = init.raw;
  if (init.body !== undefined) {
    headers['Content-Type'] = 'application/json';
    body = JSON.stringify(init.body);
  }
  let res: Response;
  try {
    res = await fetch(`/api/v1${path}`, { method, headers, body, credentials: 'same-origin' });
  } catch {
    throw new ApiError(0, 'network', 'The server is not reachable.');
  }
  if (!res.ok) {
    let code = 'error';
    let detail = res.statusText;
    try {
      const p = await res.json();
      code = p.code ?? code;
      detail = p.detail ?? p.title ?? detail;
    } catch {
      /* not a problem document */
    }
    if (res.status === 401 && path !== '/auth/login') onUnauthorized();
    throw new ApiError(res.status, code, detail);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

function qs(params: Record<string, string | number | boolean | undefined | null>): string {
  const u = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== '' && v !== false) u.set(k, String(v));
  }
  const s = u.toString();
  return s ? `?${s}` : '';
}

export interface ListParams {
  q?: string;
  unsorted?: boolean;
  /** The trash instead of the library (needs the delete scope). */
  trash?: boolean;
  sort?: 'new' | 'old' | 'used';
  limit?: number;
  cursor?: string | null;
}

export const api = {
  login: (username: string, password: string) =>
    request<{ user: { id: number; username: string } }>('POST', '/auth/login', { body: { username, password } }),
  logout: () => request<void>('POST', '/auth/logout'),
  me: () => request<Me>('GET', '/me'),
  setPrefs: (prefs: Record<string, unknown>) => request<Me>('PATCH', '/me', { body: { prefs } }),
  stats: () => request<Stats>('GET', '/stats'),

  listGifs: (p: ListParams) => request<Page<Gif>>('GET', `/gifs${qs({ ...p })}`),
  getGif: (id: string) => request<Gif>('GET', `/gifs/${id}`),
  unsortedNext: (after: string | undefined, sort: 'new' | 'old') =>
    request<{ item: Gif | null; remaining: number; total: number }>('GET', `/gifs/unsorted/next${qs({ after, sort })}`),
  setTags: (id: string, tags: string[]) => request<Gif>('PATCH', `/gifs/${id}`, { body: { tags } }),
  trash: (id: string) => request<void>('DELETE', `/gifs/${id}`),
  restore: (id: string) => request<Gif>('POST', `/gifs/${id}/restore`),
  /** All or nothing, at most 500 ids. */
  batch: (ids: string[], change: { add?: string[]; remove?: string[]; trash?: boolean }) =>
    request<{ updated: number; not_found: string[] }>('POST', '/gifs/batch', { body: { ids, ...change } }),

  listTags: (p: { q?: string; sort?: 'recent' | 'popular' | 'alpha'; limit?: number }) =>
    request<Page<Tag>>('GET', `/tags${qs({ ...p })}`),
  suggest: (selected: string[], limit = 12) =>
    request<{ recent: Tag[]; related: Tag[] }>('GET', `/tags/suggest${qs({ selected: selected.join(','), limit })}`),

  renameTag: (name: string, to: string) =>
    request<Tag>('PATCH', `/tags/${encodeURIComponent(name)}`, { body: { name: to } }),
  mergeTag: (name: string, into: string) =>
    request<Tag>('POST', `/tags/${encodeURIComponent(name)}/merge`, { body: { into } }),
  deleteTag: (name: string) => request<void>('DELETE', `/tags/${encodeURIComponent(name)}`),
  listTagsPage: (p: { q?: string; sort?: 'recent' | 'popular' | 'alpha'; limit?: number; cursor?: string | null }) =>
    request<Page<Tag>>('GET', `/tags${qs({ ...p })}`),

  listKeys: (includeRevoked: boolean) =>
    request<{ items: ApiKey[] }>('GET', `/keys${qs({ include_revoked: includeRevoked })}`),
  createKey: (b: { name: string; preset: KeyPreset; deny_tags?: string[]; rate_limit?: number | null; expires_in_days?: number }) =>
    request<ApiKey & { key: string }>('POST', '/keys', { body: b }),
  updateKey: (id: string, b: { name?: string; preset?: KeyPreset; deny_tags?: string[]; rate_limit?: number | null }) =>
    request<ApiKey>('PATCH', `/keys/${id}`, { body: b }),
  revokeKey: (id: string) => request<void>('DELETE', `/keys/${id}`),

  upload: (file: Blob, type: string) =>
    request<Gif>('POST', `/gifs${qs({ source: 'web', wait: 0 })}`, {
      raw: file,
      headers: { 'Content-Type': type || 'application/octet-stream' },
    }),
  searchProviders: () => request<{ items: SearchProvider[] }>('GET', '/search/providers'),
  search: (p: { provider: ProviderId; q?: string; cursor?: string | null; limit?: number }) =>
    request<Page<SearchItem>>('GET', `/search${qs({ ...p })}`),
  setProviderKey: (id: ProviderId, key: string | null) =>
    request<SearchProvider>('PUT', `/search/providers/${id}/key`, { body: { key } }),
  /** Download a search result into the library. It lands in Unsorted unless hashtags are given. */
  addFromSearch: (provider: ProviderId, item: SearchItem, tags: string[]) =>
    request<Gif>('POST', '/gifs', {
      body: {
        url: item.download.url,
        source: provider,
        source_id: item.id,
        description: item.title || undefined,
        tags: tags.length ? tags : undefined,
        wait: 0,
      },
    }),
  addUrl: (url: string) => request<Gif>('POST', '/gifs', { body: { url, source: 'url', wait: 0 } }),
};

/** Media URLs are absolute (built from publicUrl); the UI always loads them from its own origin. */
export function mediaPath(url: string): string {
  try {
    const u = new URL(url, location.href);
    return u.pathname + u.search;
  } catch {
    return url;
  }
}

// Content shared into the installed app (Android share target). The service worker (public/sw.js)
// parks it in Cache Storage; this reads it back and clears it.

const STASH = 'unigif-share';

export interface Shared {
  files: File[];
  urls: string[];
  /** Text arrived but held no link, so there was nothing to add. */
  ignoredText: boolean;
}

const LINK = /https?:\/\/[^\s<>"']+/i;

export async function takeShared(): Promise<Shared | null> {
  if (typeof caches === 'undefined') return null;
  let cache: Cache;
  try {
    if (!(await caches.has(STASH))) return null;
    cache = await caches.open(STASH);
  } catch {
    return null;
  }
  const requests = [...(await cache.keys())].sort((a, b) => a.url.localeCompare(b.url));
  if (requests.length === 0) return null;

  const out: Shared = { files: [], urls: [], ignoredText: false };
  for (const req of requests) {
    const res = await cache.match(req);
    await cache.delete(req);
    if (!res) continue;
    if (req.url.endsWith('-note')) {
      try {
        const note = (await res.json()) as { url?: string; text?: string; title?: string };
        // Browsers put the page address in `url`; some apps put it inside `text`.
        const link = [note.url, note.text].map((s) => s?.match(LINK)?.[0]).find(Boolean);
        if (link) {
          if (!out.urls.includes(link)) out.urls.push(link);
        } else if (out.files.length === 0) {
          out.ignoredText = true;
        }
      } catch {
        /* ignore a damaged note */
      }
    } else {
      const blob = await res.blob();
      let name = 'shared';
      try {
        name = decodeURIComponent(res.headers.get('X-Filename') ?? name);
      } catch {
        /* keep the default name */
      }
      out.files.push(new File([blob], name, { type: blob.type }));
    }
  }
  // A shared file's accompanying text is only a caption, never a reason to complain.
  if (out.files.length > 0) out.ignoredText = false;
  return out;
}

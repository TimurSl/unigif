import { api, type Gif } from './api';
import { app, emitGif, errorText, refreshStats, toast } from './app.svelte';

function isAnimation(f: File): boolean {
  return f.type.startsWith('image/') || f.type.startsWith('video/') || /\.(gif|webp|apng|mp4|m4v|mov|webm|mkv)$/i.test(f.name);
}

async function run(jobs: (() => Promise<Gif>)[]) {
  let added = 0;
  let dupes = 0;
  let failed = 0;
  app.upload = { done: 0, total: jobs.length };
  let next = 0;
  const worker = async () => {
    while (next < jobs.length) {
      const job = jobs[next++];
      try {
        const gif = await job();
        if (gif.duplicate) dupes++;
        else added++;
        emitGif('added', gif);
      } catch (e) {
        failed++;
        toast(errorText(e), { kind: 'error' });
      }
      if (app.upload) app.upload.done++;
    }
  };
  await Promise.all(Array.from({ length: Math.min(3, jobs.length) }, worker));
  app.upload = null;
  void refreshStats();
  const parts: string[] = [];
  if (added) parts.push(added === 1 ? 'Added 1 GIF' : `Added ${added} GIFs`);
  if (dupes) parts.push(dupes === 1 ? '1 was already in your library' : `${dupes} were already in your library`);
  if (parts.length && !failed) toast(parts.join(' · '));
}

export function uploadFiles(files: File[]) {
  const usable = files.filter(isAnimation);
  if (usable.length < files.length) toast('Some files were skipped: only GIFs, images and videos can be added.', { kind: 'error' });
  if (usable.length) void run(usable.map((f) => () => api.upload(f, f.type)));
}

export function uploadUrl(url: string) {
  void run([() => api.addUrl(url)]);
}

export function looksLikeUrl(text: string): boolean {
  return /^https?:\/\/\S+$/i.test(text.trim());
}

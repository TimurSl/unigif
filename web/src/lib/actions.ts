// Things you can do to a GIF from more than one place (tile menu, detail view, tag sheet).
import { api, mediaPath, type Gif } from './api';
import { emitGif, errorText, refreshStats, toast } from './app.svelte';

/** The address people paste elsewhere: the full MP4, as an absolute URL. */
export function gifUrl(g: Gif): string | null {
  return g.media?.mp4 ? new URL(mediaPath(g.media.mp4.url), location.href).href : null;
}

export async function copyText(label: string, text: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast(`${label} copied`);
  } catch {
    toast('Could not copy', { kind: 'error' });
  }
}

export const canShare = typeof navigator.share === 'function';

export async function shareGif(g: Gif) {
  const url = gifUrl(g);
  if (!url || !canShare) return;
  try {
    await navigator.share({ url });
  } catch (e) {
    // Closing the share sheet is not an error.
    if (!(e instanceof DOMException && e.name === 'AbortError')) toast('Could not share', { kind: 'error' });
  }
}

/** Move to trash with a five-second Undo. Resolves to false if the server refused. */
export async function trashGif(g: Gif): Promise<boolean> {
  try {
    await api.trash(g.id);
  } catch (e) {
    toast(errorText(e), { kind: 'error' });
    return false;
  }
  emitGif('removed', g);
  void refreshStats();
  toast('Moved to trash', {
    action: {
      label: 'Undo',
      run: () =>
        api
          .restore(g.id)
          .then((r) => {
            emitGif('added', r);
            void refreshStats();
          })
          .catch((e) => toast(errorText(e), { kind: 'error' })),
    },
    ms: 5000,
  });
  return true;
}

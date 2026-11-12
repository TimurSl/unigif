import { api, ApiError, type Gif, type ListParams } from './api';
import { toast, errorText } from './app.svelte';

/** A cursor-paginated list of GIFs that fills itself, with a guard against stale responses. */
export class Feed {
  items: Gif[] = $state([]);
  loading = $state(false);
  done = $state(false);
  error = $state('');
  private cursor: string | null = null;
  private gen = 0;
  private pollTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(private params: () => ListParams) {}

  async reset() {
    this.gen++;
    this.cursor = null;
    this.items = [];
    this.done = false;
    this.error = '';
    this.loading = false;
    await this.more();
  }

  async more() {
    if (this.loading || this.done) return;
    const gen = this.gen;
    this.loading = true;
    try {
      const page = await api.listGifs({ ...this.params(), limit: 60, cursor: this.cursor });
      if (gen !== this.gen) return;
      const have = new Set(this.items.map((g) => g.id));
      this.items.push(...page.items.filter((g) => !have.has(g.id)));
      this.cursor = page.next_cursor;
      this.done = page.next_cursor === null;
      this.schedulePoll();
    } catch (e) {
      if (gen !== this.gen) return;
      this.error = errorText(e);
      this.done = true;
      if (!(e instanceof ApiError && e.status === 401)) toast(this.error, { kind: 'error' });
    } finally {
      if (gen === this.gen) this.loading = false;
    }
  }

  replace(gif: Gif) {
    const i = this.items.findIndex((g) => g.id === gif.id);
    if (i >= 0) this.items[i] = gif;
  }

  remove(id: string) {
    const i = this.items.findIndex((g) => g.id === id);
    if (i >= 0) this.items.splice(i, 1);
  }

  has(id: string) {
    return this.items.some((g) => g.id === id);
  }

  prepend(gif: Gif) {
    if (this.has(gif.id)) return this.replace(gif);
    this.items.unshift(gif);
    this.schedulePoll();
  }

  /** Items still being transcoded are re-fetched until they are ready. */
  private schedulePoll() {
    clearTimeout(this.pollTimer);
    if (!this.items.some((g) => g.status === 'processing')) return;
    const gen = this.gen;
    this.pollTimer = setTimeout(async () => {
      const pending = this.items.filter((g) => g.status === 'processing').slice(0, 20);
      await Promise.all(
        pending.map(async (g) => {
          try {
            const fresh = await api.getGif(g.id);
            if (gen === this.gen && fresh.status !== 'processing') this.replace(fresh);
          } catch {
            /* trashed or hidden meanwhile */
          }
        }),
      );
      if (gen === this.gen) this.schedulePoll();
    }, 1500);
  }

  destroy() {
    this.gen++;
    clearTimeout(this.pollTimer);
  }
}

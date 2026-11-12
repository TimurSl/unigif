import type { Gif } from './api';

/** Which GIFs are ticked in a list, with Shift-click ranges. */
export class Selection {
  ids = $state(new Set<string>());
  /** Whether select mode is on: tiles toggle instead of opening. */
  active = $state(false);
  private anchor: string | null = null;

  get count() {
    return this.ids.size;
  }

  has(id: string) {
    return this.ids.has(id);
  }

  start(id?: string) {
    this.active = true;
    if (id) this.toggle(id);
  }

  toggle(id: string) {
    const next = new Set(this.ids);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    this.ids = next;
    this.anchor = id;
  }

  /** Shift-click: tick everything between the last tile clicked and this one. */
  range(items: Gif[], id: string) {
    const to = items.findIndex((g) => g.id === id);
    const from = this.anchor ? items.findIndex((g) => g.id === this.anchor) : -1;
    if (to < 0 || from < 0) return this.toggle(id);
    const next = new Set(this.ids);
    for (let i = Math.min(from, to); i <= Math.max(from, to); i++) next.add(items[i].id);
    this.ids = next;
    this.anchor = id;
  }

  all(items: Gif[]) {
    this.ids = new Set(items.map((g) => g.id));
  }

  /** Forget ids that are no longer in the list (trashed, moved elsewhere). */
  keepOnly(items: Gif[]) {
    const have = new Set(items.map((g) => g.id));
    const next = new Set([...this.ids].filter((id) => have.has(id)));
    if (next.size !== this.ids.size) this.ids = next;
  }

  clear() {
    this.ids = new Set();
    this.anchor = null;
  }

  stop() {
    this.clear();
    this.active = false;
  }
}

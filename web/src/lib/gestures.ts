// Touch gestures for the tag sheet (plan 9.5): swipe left to skip, right to go back, down to close.

export interface SwipeHandlers {
  left?: () => void;
  right?: () => void;
  down?: () => void;
}

const MIN_DISTANCE = 60;
const MIN_DOWN = 80;
const DOMINANCE = 1.5;

/**
 * Svelte action. Mouse is ignored (a desktop drag is not a swipe). The node should set
 * `touch-action: none` so the browser does not claim the gesture for scrolling.
 */
export function swipe(node: HTMLElement, initial: SwipeHandlers) {
  let handlers = initial;
  let start: { id: number; x: number; y: number } | null = null;

  const down = (e: PointerEvent) => {
    if (e.pointerType === 'mouse' || !e.isPrimary) return;
    start = { id: e.pointerId, x: e.clientX, y: e.clientY };
  };
  const up = (e: PointerEvent) => {
    if (!start || e.pointerId !== start.id) return;
    const dx = e.clientX - start.x;
    const dy = e.clientY - start.y;
    start = null;
    if (Math.abs(dx) >= MIN_DISTANCE && Math.abs(dx) > DOMINANCE * Math.abs(dy)) {
      (dx < 0 ? handlers.left : handlers.right)?.();
    } else if (dy >= MIN_DOWN && dy > DOMINANCE * Math.abs(dx)) {
      handlers.down?.();
    }
  };
  const cancel = () => (start = null);

  node.addEventListener('pointerdown', down);
  node.addEventListener('pointerup', up);
  node.addEventListener('pointercancel', cancel);
  return {
    update(next: SwipeHandlers) {
      handlers = next;
    },
    destroy() {
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointerup', up);
      node.removeEventListener('pointercancel', cancel);
    },
  };
}

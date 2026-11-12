// Full-screen sheets on a phone must end exactly above the on-screen keyboard. The keyboard shrinks the
// *visual* viewport but not always the layout viewport (iOS, and Chrome's default), so sheets size
// themselves from `--vv-height` / `--vv-top`, which this keeps current.

/** Start tracking; returns a function that stops and clears the variables. */
export function trackVisualViewport(): () => void {
  const vv = window.visualViewport;
  const root = document.documentElement;
  if (!vv) return () => {};
  const apply = () => {
    root.style.setProperty('--vv-height', `${vv.height}px`);
    root.style.setProperty('--vv-top', `${vv.offsetTop}px`);
  };
  apply();
  vv.addEventListener('resize', apply);
  vv.addEventListener('scroll', apply);
  return () => {
    vv.removeEventListener('resize', apply);
    vv.removeEventListener('scroll', apply);
    root.style.removeProperty('--vv-height');
    root.style.removeProperty('--vv-top');
  };
}

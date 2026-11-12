// Installable-app plumbing: the service worker (Android share target) and the install prompt.

interface InstallPrompt extends Event {
  prompt: () => Promise<void>;
  userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
}

export const pwa = $state({
  /** Set when the browser offers to install the app (Chromium); null otherwise. */
  installPrompt: null as InstallPrompt | null,
  /** Running as an installed app (standalone window). */
  standalone: matchMedia('(display-mode: standalone)').matches || (navigator as { standalone?: boolean }).standalone === true,
});

window.addEventListener('beforeinstallprompt', (e) => {
  e.preventDefault();
  pwa.installPrompt = e as InstallPrompt;
});
window.addEventListener('appinstalled', () => {
  pwa.installPrompt = null;
  pwa.standalone = true;
});

export async function install() {
  const p = pwa.installPrompt;
  if (!p) return;
  pwa.installPrompt = null;
  await p.prompt();
  await p.userChoice.catch(() => {});
}

/** iOS Safari has no install prompt and no share target; the Settings page explains the manual route. */
export const isIos = /iphone|ipad|ipod/i.test(navigator.userAgent);

export function registerServiceWorker() {
  // Not in `vite dev`: a worker on the dev server would outlive it and confuse hot reloading.
  if (!import.meta.env.PROD || !('serviceWorker' in navigator)) return;
  addEventListener('load', () => {
    navigator.serviceWorker.register('/sw.js', { scope: '/' }).catch(() => {
      /* no worker, no share target; everything else works */
    });
  });
}

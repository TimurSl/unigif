// Small display helpers shared by the screens.

/** `183 kB`, `2.1 MB`: decimal units, like the server and the plan. */
export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} MB`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} kB`;
  return `${Math.max(0, Math.round(n))} B`;
}

/** "Firefox on Linux" from a User-Agent string; good enough to tell sessions apart. */
export function describeDevice(ua: string | null): string {
  if (!ua) return 'Unknown device';
  const browser =
    /Edg\//.test(ua) ? 'Edge'
    : /OPR\/|Opera/.test(ua) ? 'Opera'
    : /Firefox\/|FxiOS/.test(ua) ? 'Firefox'
    : /Chrome\/|CriOS/.test(ua) ? 'Chrome'
    : /Safari\//.test(ua) ? 'Safari'
    : /curl\//.test(ua) ? 'curl'
    : null;
  const os =
    /iPhone/.test(ua) ? 'iPhone'
    : /iPad/.test(ua) ? 'iPad'
    : /Android/.test(ua) ? 'Android'
    : /Mac OS X|Macintosh/.test(ua) ? 'macOS'
    : /Windows/.test(ua) ? 'Windows'
    : /CrOS/.test(ua) ? 'ChromeOS'
    : /Linux/.test(ua) ? 'Linux'
    : null;
  if (browser && os) return `${browser} on ${os}`;
  return browser ?? os ?? ua.slice(0, 40);
}

/** "just now", "5 min ago", "3 h ago", "2 days ago", then a date. */
export function ago(iso: string, now = Date.now()): string {
  const s = Math.max(0, (now - new Date(iso).getTime()) / 1000);
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  if (s < 86400 * 14) {
    const d = Math.floor(s / 86400);
    return `${d} ${d === 1 ? 'day' : 'days'} ago`;
  }
  return new Date(iso).toLocaleDateString();
}

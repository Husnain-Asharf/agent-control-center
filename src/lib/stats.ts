import { useEffect, useState } from 'react';
import { request } from './ipc';
import type { Stats } from './types';

/** `512 KB`, `3.4 MB`, `1.2 GB`. Null for an unknown size. */
export function fmtBytes(n: number | null | undefined): string | null {
  if (n == null || !Number.isFinite(n)) return null;
  if (n < 1024 * 1024) return `${Math.max(1, Math.round(n / 1024))} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(1)} GB`;
}

/** The daemon's stats, asked every `everyMs` while the screen is open. Null until the first answer. */
export function useStats(everyMs = 5000): Stats | null {
  const [stats, setStats] = useState<Stats | null>(null);
  useEffect(() => {
    let live = true;
    const ask = () =>
      request<Stats>('stats')
        .then((s) => live && setStats(s))
        .catch(() => {});
    ask();
    const t = setInterval(ask, everyMs);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [everyMs]);
  return stats;
}

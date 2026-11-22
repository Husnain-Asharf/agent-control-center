const pad = (n: number): string => String(n).padStart(2, '0');

/** Two-letter badge for a project name: `atlas-api` gives `AA`, `atlas` gives `AT`. */
export function monogram(name: string): string {
  const words = name.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  if (words.length === 0) return '?';
  const letters = words.length === 1 ? Array.from(words[0]).slice(0, 2) : words.slice(0, 2).map((w) => Array.from(w)[0]);
  return letters.join('').toUpperCase();
}

/** `42s`, `12m`, `1h 05m`. */
export function fmtDur(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  return m < 60 ? `${m}m` : `${Math.floor(m / 60)}h ${pad(m % 60)}m`;
}

/** `4:12`, or `1:02:05` from one hour. */
export function fmtTimer(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h > 0 ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}

/** Local time as `HH:MM`, 24-hour. */
export function clock(ts: number): string {
  const d = new Date(ts);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** `.worktrees/<slug>` for a session's worktree path, which has backslashes on Windows. */
export function worktreeLabel(path: string): string {
  return `.worktrees/${path.split(/[\\/]/).filter(Boolean).pop() ?? ''}`;
}

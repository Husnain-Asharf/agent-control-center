import type { Session, StatusChange } from './types';

export type Segment = { kind: 'work' | 'needs' | 'idle'; start: number; end: number | null };

const KIND = {
  starting: 'work',
  working: 'work',
  needs_you: 'needs',
  idle: 'idle',
} as const;

/**
 * Turns a status history into lane segments inside the window `[now - windowMs, now]`.
 * Done, Failed and Stopped entries are not segments (Done and Failed are marks).
 * The last entry stays open (`end: null`) unless a terminal entry closes it.
 */
export function segmentsFor(history: StatusChange[], now: number, windowMs: number): Segment[] {
  const from = now - windowMs;
  const out: Segment[] = [];
  history.forEach((entry, i) => {
    if (!(entry.status in KIND)) return;
    const kind = KIND[entry.status as keyof typeof KIND];
    const next = history[i + 1];
    const end = next ? next.at : null;
    if ((end ?? now) <= from) return;
    out.push({ kind, start: Math.max(entry.at, from), end });
  });
  return out;
}

export function marksFor(session: Session): { kind: 'done' | 'failed'; at: number } | null {
  if (session.status !== 'done' && session.status !== 'failed') return null;
  return { kind: session.status, at: session.ended_at ?? session.status_since };
}

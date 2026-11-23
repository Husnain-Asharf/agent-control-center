import type { Session } from './types';

/** Dollars with two decimals, or cents under a dollar: `$2.35`, `$0.40`. */
export function fmtCost(usd: number): string {
  return `$${(Number.isFinite(usd) ? usd : 0).toFixed(2)}`;
}

/** How a share of a limit reads: `ok` below 60%, `warn` below 85%, then `high`. */
export function level(pct: number | null | undefined): 'ok' | 'warn' | 'high' {
  if (pct == null || pct < 60) return 'ok';
  return pct < 85 ? 'warn' : 'high';
}

/** When a limit window resets, from `now`: `in 2 h 10 min`, `in 3 d`, or `now` when past. */
export function resetText(resetsAt: number | null | undefined, now: number): string | null {
  if (resetsAt == null) return null;
  const min = Math.round((resetsAt * 1000 - now) / 60_000);
  if (min <= 0) return 'now';
  if (min < 60) return `in ${min} min`;
  const h = Math.floor(min / 60);
  if (h < 24) return `in ${h} h${min % 60 ? ` ${min % 60} min` : ''}`;
  return `in ${Math.round(h / 24)} d`;
}

export interface ProjectUsage {
  projectId: string;
  costUsd: number;
  sessions: number;
}

/** Each project's sessions that reported use, with their total cost, highest first. */
export function usageByProject(sessions: Session[]): ProjectUsage[] {
  const by = new Map<string, ProjectUsage>();
  for (const s of sessions) {
    if (!s.usage) continue;
    const row = by.get(s.project_id) ?? { projectId: s.project_id, costUsd: 0, sessions: 0 };
    row.costUsd += s.usage.cost_usd;
    row.sessions += 1;
    by.set(s.project_id, row);
  }
  return [...by.values()].sort((a, b) => b.costUsd - a.costUsd || a.projectId.localeCompare(b.projectId));
}

/** The short usage line for a session row: `$2.35 · context 64%`. Null before any report. */
export function usageLine(s: Pick<Session, 'usage'>): string | null {
  if (!s.usage) return null;
  const ctx = s.usage.context_used_percentage;
  return ctx == null ? fmtCost(s.usage.cost_usd) : `${fmtCost(s.usage.cost_usd)} · context ${Math.round(ctx)}%`;
}

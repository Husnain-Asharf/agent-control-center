import type { PlanUsage, SessionStatus } from './types';

export const COMPACT_AT = 70;
export const HIGH_MEMORY_BYTES = 1.5 * 1024 ** 3;
export const LONG_IDLE_MS = 2 * 3_600_000;
export const OFTEN_WAITS_MS = 10 * 60_000;
export const PLAN_HIGH_AT = 85;

export type RecId = 'compact' | 'memory' | 'stop' | 'waits';

export interface Rec {
  id: RecId;
  label: string;
  /** Why, for the title text. */
  detail: string;
}

export interface RecInput {
  status: SessionStatus;
  contextPct: number | null;
  memoryBytes: number | null;
  /** How long it has been idle now, or 0 when it is not idle. */
  idleMs: number;
  /** How long it waited for you in the range. */
  waitedMs: number;
}

const RUNNING: SessionStatus[] = ['starting', 'working', 'needs_you', 'idle'];

export const isRunning = (status: SessionStatus): boolean => RUNNING.includes(status);

/** The rules for one session, in the order they are shown. Only running sessions get any. */
export function recommend(s: RecInput): Rec[] {
  if (!isRunning(s.status)) return [];
  const out: Rec[] = [];
  if (s.contextPct != null && s.contextPct >= COMPACT_AT) {
    out.push({
      id: 'compact',
      label: 'Compact',
      detail: 'Context is close to being compacted automatically. Compacting now keeps it in your control.',
    });
  }
  if (s.memoryBytes != null && s.memoryBytes > HIGH_MEMORY_BYTES) {
    out.push({ id: 'memory', label: 'High memory', detail: 'Uses more than 1.5 GB of memory.' });
  }
  if (s.status === 'idle' && s.idleMs > LONG_IDLE_MS) {
    out.push({ id: 'stop', label: 'Stop if done', detail: 'Idle for more than 2 hours.' });
  }
  if (s.waitedMs > OFTEN_WAITS_MS) {
    out.push({ id: 'waits', label: 'Often waits for you', detail: 'Waited for you more than 10 minutes in this range.' });
  }
  return out;
}

/** True when the plan's 5-hour use is high enough to warn about. */
export function planHigh(plan: PlanUsage | null | undefined): boolean {
  const pct = plan?.five_hour?.used_percentage;
  return pct != null && pct >= PLAN_HIGH_AT;
}

/** Whether `/compact` can be sent now, and why not. */
export function canCompact(status: SessionStatus): { ok: boolean; why: string | null } {
  if (status === 'idle') return { ok: true, why: null };
  if (status === 'working' || status === 'starting') return { ok: false, why: 'Working. Compact once it is idle.' };
  if (status === 'needs_you') return { ok: false, why: 'Waiting on you. Answer it first.' };
  return { ok: false, why: 'Not running.' };
}

// Pure parts of the Analytics screen: totals for the tiles, series for the charts and rows for the table.
import { recommend, type Rec } from './recommend';
import { segmentsFor } from './segments';
import type { Approval, EventRecord, Project, Session, SessionStatus, Stats, UsageSample } from './types';

export const RANGE_IDS = ['1h', '24h', '7d'] as const;
export type RangeId = (typeof RANGE_IDS)[number];

export const RANGE_MS: Record<RangeId, number> = {
  '1h': 3_600_000,
  '24h': 24 * 3_600_000,
  '7d': 7 * 24 * 3_600_000,
};

export const RANGE_LABEL: Record<RangeId, string> = { '1h': '1h', '24h': '24h', '7d': '7d' };

/** Samples come once a minute. A longer step between two means nothing ran in between. */
export const GAP_MS = 3 * 60_000;

export interface Point {
  t: number;
  v: number;
}

export interface Series {
  id: string;
  label: string;
  /** Unbroken stretches. A gap in the samples starts a new one. */
  runs: Point[][];
}

const RUNNING: SessionStatus[] = ['starting', 'working', 'needs_you', 'idle'];

const inRange = (samples: UsageSample[], since: number): UsageSample[] =>
  samples.filter((s) => s.ts >= since).sort((a, b) => a.ts - b.ts);

/** Splits points into runs wherever two neighbours are more than `maxGap` apart. */
export function splitRuns(points: Point[], maxGap = GAP_MS): Point[][] {
  const runs: Point[][] = [];
  let cur: Point[] = [];
  for (const p of points) {
    if (cur.length && p.t - cur[cur.length - 1].t > maxGap) {
      runs.push(cur);
      cur = [];
    }
    cur.push(p);
  }
  if (cur.length) runs.push(cur);
  return runs;
}

/** Sessions that are running now. */
export const runningCount = (sessions: Session[]): number => sessions.filter((s) => RUNNING.includes(s.status)).length;

/** Memory all sessions use now, or null when none could be measured. */
export function memoryNow(stats: Stats | null): number | null {
  const known = (stats?.sessions ?? []).filter((r) => r.memory_bytes != null);
  return known.length ? known.reduce((n, r) => n + (r.memory_bytes ?? 0), 0) : null;
}

/**
 * What each session cost inside the range. A session that started before the range counts
 * from its first sample in the range; one that started inside it counts from zero.
 * The session's live cost is used when it is newer than its last sample.
 */
export function costInRange(samples: UsageSample[], sessions: Record<string, Session>, since: number): Map<string, number> {
  const first = new Map<string, number>();
  const last = new Map<string, number>();
  for (const sample of inRange(samples, since)) {
    for (const r of sample.sessions) {
      if (!first.has(r.session_id)) first.set(r.session_id, r.cost_usd);
      last.set(r.session_id, r.cost_usd);
    }
  }
  const out = new Map<string, number>();
  for (const [id, f] of first) {
    const s = sessions[id];
    const base = s && (s.first_started_at ?? s.started_at) >= since ? 0 : f;
    const end = Math.max(last.get(id) ?? f, s?.usage?.cost_usd ?? 0);
    out.set(id, Math.max(0, end - base));
  }
  for (const s of Object.values(sessions)) {
    if (!out.has(s.id) && s.usage && (s.first_started_at ?? s.started_at) >= since) out.set(s.id, Math.max(0, s.usage.cost_usd));
  }
  return out;
}

export const sumValues = (m: Map<string, number>): number => [...m.values()].reduce((a, b) => a + b, 0);

/** The plan's 5-hour and weekly use over time. */
export function planSeries(samples: UsageSample[], since: number): Series[] {
  const list = inRange(samples, since);
  const pick = (key: 'five_hour' | 'seven_day') =>
    splitRuns(list.filter((s) => s[key] != null).map((s) => ({ t: s.ts, v: s[key] as number })));
  return [
    { id: 'five_hour', label: '5-hour', runs: pick('five_hour') },
    { id: 'seven_day', label: 'Weekly', runs: pick('seven_day') },
  ];
}

/** Memory all running sessions used at each sample, where it was measured. */
export function memorySeries(samples: UsageSample[], since: number): Series {
  const points: Point[] = [];
  for (const s of inRange(samples, since)) {
    const known = s.sessions.filter((r) => r.memory_bytes != null);
    if (known.length) points.push({ t: s.ts, v: known.reduce((n, r) => n + (r.memory_bytes ?? 0), 0) });
  }
  return { id: 'memory', label: 'Memory', runs: splitRuns(points) };
}

/** How many projects get their own cost line. The rest share one line, Other. */
export const MAX_PROJECT_LINES = 4;

/**
 * Cost added since the start of the range, at each sample. Cost only grows while a session
 * runs, so the line holds its value across gaps and is drawn unbroken.
 * `by: 'project'` gives a line per project (the top ones, then Other), highest first.
 */
export function costSeries(
  samples: UsageSample[],
  sessions: Record<string, Session>,
  projects: Record<string, Project>,
  since: number,
  by: 'total' | 'project',
): Series[] {
  const list = inRange(samples, since);
  const base = new Map<string, number>();
  const last = new Map<string, number>();
  const groupOf = (id: string) => (by === 'total' ? 'total' : (sessions[id]?.project_id ?? 'unknown'));
  const groups = new Set<string>();
  const rows: { t: number; sums: Map<string, number> }[] = [];
  for (const sample of list) {
    for (const r of sample.sessions) {
      if (!base.has(r.session_id)) {
        const s = sessions[r.session_id];
        base.set(r.session_id, s && (s.first_started_at ?? s.started_at) >= since ? 0 : r.cost_usd);
      }
      last.set(r.session_id, r.cost_usd);
      groups.add(groupOf(r.session_id));
    }
    const sums = new Map<string, number>();
    for (const [id, c] of last) {
      const g = groupOf(id);
      sums.set(g, (sums.get(g) ?? 0) + Math.max(0, c - (base.get(id) ?? 0)));
    }
    rows.push({ t: sample.ts, sums });
  }
  if (by === 'total') {
    return [{ id: 'total', label: 'Total', runs: rows.length ? [rows.map((r) => ({ t: r.t, v: r.sums.get('total') ?? 0 }))] : [] }];
  }
  const final = rows.length ? rows[rows.length - 1].sums : new Map<string, number>();
  const ranked = [...groups].sort((a, b) => (final.get(b) ?? 0) - (final.get(a) ?? 0) || a.localeCompare(b));
  const own = ranked.length > MAX_PROJECT_LINES ? ranked.slice(0, MAX_PROJECT_LINES - 1) : ranked;
  const rest = ranked.filter((g) => !own.includes(g));
  const name = (g: string) => projects[g]?.name ?? 'Unknown project';
  const out: Series[] = own.map((g) => ({ id: g, label: name(g), runs: [rows.map((r) => ({ t: r.t, v: r.sums.get(g) ?? 0 }))] }));
  if (rest.length) {
    out.push({
      id: 'other',
      label: 'Other',
      runs: [rows.map((r) => ({ t: r.t, v: rest.reduce((n, g) => n + (r.sums.get(g) ?? 0), 0) }))],
    });
  }
  return out;
}

/** The series' value at `t`: the nearest point at or before it inside the same run, or null in a gap. */
export function valueAt(series: Series, t: number, maxGap = GAP_MS): number | null {
  for (const run of series.runs) {
    if (!run.length || t < run[0].t - maxGap / 2 || t > run[run.length - 1].t + maxGap / 2) continue;
    let best = run[0];
    for (const p of run) if (Math.abs(p.t - t) < Math.abs(best.t - t)) best = p;
    return best.v;
  }
  return null;
}

/** Every sample time across the series, sorted, for the crosshair to snap to. */
export function sampleTimes(series: Series[]): number[] {
  const set = new Set<number>();
  for (const s of series) for (const run of s.runs) for (const p of run) set.add(p.t);
  return [...set].sort((a, b) => a - b);
}

/** The time in `times` closest to `t`, or null when there are none. */
export function nearest(times: number[], t: number): number | null {
  if (!times.length) return null;
  let lo = 0;
  let hi = times.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (times[mid] < t) lo = mid + 1;
    else hi = mid;
  }
  const a = times[Math.max(0, lo - 1)];
  const b = times[lo];
  return Math.abs(a - t) <= Math.abs(b - t) ? a : b;
}

export const hasPoints = (series: Series[]): boolean => series.some((s) => s.runs.some((r) => r.length > 0));

export const maxValue = (series: Series[]): number =>
  series.reduce((m, s) => s.runs.reduce((m2, r) => r.reduce((m3, p) => Math.max(m3, p.v), m2), m), 0);

/** A round top for an axis: 1, 2, 2.5 or 5 times a power of ten, at or above `v`. */
export function niceMax(v: number): number {
  if (!(v > 0)) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 2, 2.5, 5, 10]) if (m * p >= v) return m * p;
  return 10 * p;
}

/** A round axis top for a size in bytes, so its quarters are round MB or GB too. */
export function niceBytes(v: number): number {
  const unit = v >= 1024 ** 3 ? 1024 ** 3 : 1024 ** 2;
  const n = v / unit;
  if (!(n > 0)) return unit;
  const p = 10 ** Math.floor(Math.log10(n));
  for (const m of [1, 2, 4, 6, 8, 10]) if (m * p >= n) return m * p * unit;
  return 10 * p * unit;
}

/** Evenly spaced times for the table view, newest last. */
export function tableTimes(since: number, now: number, count: number): number[] {
  const step = (now - since) / count;
  return Array.from({ length: count }, (_, i) => Math.round(since + step * (i + 1)));
}

export interface SessionRow {
  id: string;
  title: string;
  projectId: string;
  projectName: string;
  status: SessionStatus;
  cost: number | null;
  costInRange: number | null;
  contextPct: number | null;
  memory: number | null;
  disk: number | null;
  linesAdded: number | null;
  linesRemoved: number | null;
  model: string | null;
  workedMs: number;
  waitedMs: number;
  idleMs: number;
  asked: number;
  recs: Rec[];
}

/** How many approvals each session asked for inside the range, from the event log and the store. */
export function approvalsAsked(events: EventRecord[], approvals: Approval[], since: number): Map<string, number> {
  const seen = new Map<string, Set<string>>();
  const add = (sessionId: string, key: string) => {
    const set = seen.get(sessionId) ?? new Set<string>();
    set.add(key);
    seen.set(sessionId, set);
  };
  for (const e of events) {
    if (e.kind !== 'approval.requested' || e.ts < since || !e.session_id) continue;
    const id = (e.payload as { approval_id?: unknown } | null)?.approval_id;
    add(e.session_id, typeof id === 'string' ? id : `seq:${e.seq}`);
  }
  for (const a of approvals) if (a.created_at >= since) add(a.session_id, a.id);
  return new Map([...seen].map(([k, v]) => [k, v.size]));
}

export interface RowInput {
  sessions: Session[];
  projects: Record<string, Project>;
  stats: Stats | null;
  asked: Map<string, number>;
  costs: Map<string, number>;
  now: number;
  rangeMs: number;
}

export function buildRows({ sessions, projects, stats, asked, costs, now, rangeMs }: RowInput): SessionRow[] {
  const byId = new Map((stats?.sessions ?? []).map((r) => [r.session_id, r]));
  return sessions.map((s) => {
    const st = byId.get(s.id);
    const segs = segmentsFor(s.history, now, rangeMs);
    const total = (kind: 'work' | 'needs') =>
      segs.filter((g) => g.kind === kind).reduce((n, g) => n + ((g.end ?? now) - g.start), 0);
    const disk = st && (st.transcript_bytes != null || st.folder_bytes != null) ? (st.transcript_bytes ?? 0) + (st.folder_bytes ?? 0) : null;
    const contextPct = s.usage?.context_used_percentage ?? null;
    const memory = st?.memory_bytes ?? null;
    const idleMs = s.status === 'idle' ? Math.max(0, now - s.status_since) : 0;
    const waitedMs = total('needs');
    return {
      id: s.id,
      title: s.title,
      projectId: s.project_id,
      projectName: projects[s.project_id]?.name ?? 'Unknown project',
      status: s.status,
      cost: s.usage?.cost_usd ?? null,
      costInRange: costs.get(s.id) ?? null,
      contextPct,
      memory,
      disk,
      linesAdded: s.usage?.lines_added ?? null,
      linesRemoved: s.usage?.lines_removed ?? null,
      model: s.usage?.model ?? null,
      workedMs: total('work'),
      waitedMs,
      idleMs,
      asked: asked.get(s.id) ?? 0,
      recs: recommend({ status: s.status, contextPct, memoryBytes: memory, idleMs, waitedMs }),
    };
  });
}

export type SortKey = 'cost' | 'contextPct' | 'memory' | 'disk' | 'lines' | 'workedMs' | 'waitedMs' | 'asked';

const sortValue = (r: SessionRow, key: SortKey): number | null =>
  key === 'lines' ? (r.linesAdded == null ? null : r.linesAdded + (r.linesRemoved ?? 0)) : r[key];

/** Sorts by a numeric column. Rows without a value go last either way. */
export function sortRows(rows: SessionRow[], key: SortKey, dir: 'asc' | 'desc'): SessionRow[] {
  const sign = dir === 'asc' ? 1 : -1;
  return [...rows].sort((a, b) => {
    const x = sortValue(a, key);
    const y = sortValue(b, key);
    if (x == null && y == null) return a.title.localeCompare(b.title);
    if (x == null) return 1;
    if (y == null) return -1;
    return (x - y) * sign || a.title.localeCompare(b.title);
  });
}

/** Idle running sessions that the Compact rule picked, for Compact all. */
export const compactTargets = (rows: SessionRow[]): SessionRow[] =>
  rows.filter((r) => r.status === 'idle' && r.recs.some((x) => x.id === 'compact'));

/** Axis times inside `[since, now]`, on local round times: every 10 minutes, 4 hours or day. */
export function xTicks(since: number, now: number, range: RangeId): number[] {
  const d = new Date(since);
  d.setSeconds(0, 0);
  const step = (next: Date) => {
    if (range === '1h') next.setMinutes(next.getMinutes() + 10);
    else if (range === '24h') next.setHours(next.getHours() + 4);
    else next.setDate(next.getDate() + 1);
  };
  if (range === '1h') d.setMinutes(Math.ceil(d.getMinutes() / 10) * 10);
  else if (range === '24h') d.setHours(Math.ceil((d.getHours() + (d.getMinutes() ? 1 : 0)) / 4) * 4, 0);
  else {
    if (d.getHours() || d.getMinutes()) d.setDate(d.getDate() + 1);
    d.setHours(0, 0);
  }
  const out: number[] = [];
  for (; d.getTime() <= now; step(d)) if (d.getTime() >= since) out.push(d.getTime());
  return out;
}

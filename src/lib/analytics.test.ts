import { describe, expect, it } from 'vitest';
import {
  approvalsAsked,
  buildRows,
  compactTargets,
  costInRange,
  costSeries,
  memoryNow,
  memorySeries,
  nearest,
  niceBytes,
  niceMax,
  planSeries,
  runningCount,
  sampleTimes,
  sortRows,
  splitRuns,
  valueAt,
  xTicks,
  type SessionRow,
} from './analytics';
import type { Approval, EventRecord, Project, Session, Stats, UsageSample } from './types';

const MIN = 60_000;

function session(id: string, over: Partial<Session> = {}): Session {
  return {
    id,
    project_id: 'p1',
    agent: 'claude',
    title: id,
    branch: 'main',
    worktree: '/w',
    status: 'idle',
    status_since: 0,
    started_at: 0,
    ended_at: null,
    exit_code: null,
    history: [],
    ...over,
  };
}

const sample = (ts: number, rows: [string, number, number | null][], five: number | null = null, week: number | null = null): UsageSample => ({
  ts,
  five_hour: five,
  seven_day: week,
  sessions: rows.map(([session_id, cost_usd, memory_bytes]) => ({ session_id, cost_usd, context_used_percentage: null, memory_bytes })),
});

const projects: Record<string, Project> = {
  p1: { id: 'p1', name: 'atlas', path: '/a', host: null, base_branch: 'main', added_at: 0 },
  p2: { id: 'p2', name: 'beacon', path: '/b', host: null, base_branch: 'main', added_at: 0 },
};

describe('splitRuns', () => {
  it('breaks where samples are more than 3 minutes apart', () => {
    const pts = [0, 1, 2, 10, 11].map((m) => ({ t: m * MIN, v: m }));
    expect(splitRuns(pts).map((r) => r.map((p) => p.v))).toEqual([[0, 1, 2], [10, 11]]);
    expect(splitRuns([])).toEqual([]);
  });
});

describe('tiles', () => {
  it('counts running sessions', () => {
    expect(runningCount([session('a'), session('b', { status: 'done' }), session('c', { status: 'working' })])).toBe(2);
  });
  it('adds memory that was measured', () => {
    const stats: Stats = {
      db_bytes: 1,
      data_bytes: 2,
      sessions: [
        { session_id: 'a', memory_bytes: 100, processes: 1, scrollback_bytes: 0, transcript_bytes: null, folder_bytes: null },
        { session_id: 'b', memory_bytes: null, processes: 1, scrollback_bytes: 0, transcript_bytes: null, folder_bytes: null },
        { session_id: 'c', memory_bytes: 50, processes: 1, scrollback_bytes: 0, transcript_bytes: null, folder_bytes: null },
      ],
    };
    expect(memoryNow(stats)).toBe(150);
    expect(memoryNow({ ...stats, sessions: [stats.sessions[1]] })).toBeNull();
    expect(memoryNow(null)).toBeNull();
  });
});

describe('costInRange', () => {
  const since = 100 * MIN;
  it('counts an older session from its first sample in the range', () => {
    const sessions = { a: session('a', { started_at: 0 }) };
    const m = costInRange([sample(90 * MIN, [['a', 1, null]]), sample(101 * MIN, [['a', 2, null]]), sample(102 * MIN, [['a', 3.5, null]])], sessions, since);
    expect(m.get('a')).toBeCloseTo(1.5);
  });
  it('counts a session that started in the range from zero, with its live cost', () => {
    const sessions = {
      b: session('b', { started_at: 101 * MIN, usage: { cost_usd: 4, context_used_percentage: null, context_window_size: null, lines_added: 0, lines_removed: 0, model: null, effort: null, updated_at: 0 } }),
    };
    expect(costInRange([sample(102 * MIN, [['b', 3, null]])], sessions, since).get('b')).toBe(4);
    expect(costInRange([], sessions, since).get('b')).toBe(4);
  });
  it('gives nothing for a session with no samples that started before the range', () => {
    expect(costInRange([], { a: session('a') }, since).size).toBe(0);
  });
});

describe('planSeries', () => {
  it('keeps known values and breaks at gaps', () => {
    const s = planSeries([sample(0, [], 10, 40), sample(MIN, [], 12, null), sample(20 * MIN, [], 30, 41)], 0);
    expect(s[0].runs.map((r) => r.map((p) => p.v))).toEqual([[10, 12], [30]]);
    expect(s[1].runs.map((r) => r.map((p) => p.v))).toEqual([[40], [41]]);
  });
});

describe('memorySeries', () => {
  it('adds the measured memory at each sample', () => {
    const s = memorySeries([sample(0, [['a', 0, 10], ['b', 0, 5]]), sample(MIN, [['a', 0, null]])], 0);
    expect(s.runs).toEqual([[{ t: 0, v: 15 }]]);
  });
});

describe('costSeries', () => {
  const sessions = { a: session('a', { project_id: 'p1', started_at: -MIN }), b: session('b', { project_id: 'p2', started_at: 5 * MIN }) };
  const samples = [sample(MIN, [['a', 1, null]]), sample(6 * MIN, [['a', 2, null], ['b', 0.5, null]]), sample(20 * MIN, [['b', 3, null]])];
  it('adds cost from the start of the range and holds it across gaps', () => {
    const [total] = costSeries(samples, sessions, projects, 0, 'total');
    expect(total.runs).toHaveLength(1);
    expect(total.runs[0].map((p) => p.v)).toEqual([0, 1.5, 4]);
  });
  it('gives a line per project, highest first', () => {
    const s = costSeries(samples, sessions, projects, 0, 'project');
    expect(s.map((x) => x.label)).toEqual(['beacon', 'atlas']);
    expect(s[0].runs[0].map((p) => p.v)).toEqual([0, 0.5, 3]);
    expect(s[1].runs[0].map((p) => p.v)).toEqual([0, 1, 1]);
  });
  it('folds projects past the fourth into Other', () => {
    const many: Record<string, Session> = {};
    const rows: [string, number, number | null][] = [];
    for (let i = 1; i <= 6; i++) {
      many[`s${i}`] = session(`s${i}`, { project_id: `q${i}`, started_at: 0 });
      rows.push([`s${i}`, i, null]);
    }
    const s = costSeries([sample(MIN, rows)], many, {}, 0, 'project');
    expect(s.map((x) => x.id)).toEqual(['q6', 'q5', 'q4', 'other']);
    expect(s[3].runs[0][0].v).toBe(6);
  });
  it('is empty without samples', () => {
    expect(costSeries([], sessions, projects, 0, 'total')[0].runs).toEqual([]);
  });
});

describe('chart helpers', () => {
  const series = { id: 'x', label: 'x', runs: [[{ t: 0, v: 1 }, { t: MIN, v: 2 }], [{ t: 10 * MIN, v: 5 }]] };
  it('reads a value near a time, and null in a gap', () => {
    expect(valueAt(series, MIN)).toBe(2);
    expect(valueAt(series, 5 * MIN)).toBeNull();
    expect(valueAt(series, 10 * MIN)).toBe(5);
  });
  it('snaps to the nearest sample time', () => {
    const times = sampleTimes([series]);
    expect(times).toEqual([0, MIN, 10 * MIN]);
    expect(nearest(times, 40_000)).toBe(MIN);
    expect(nearest(times, 20 * MIN)).toBe(10 * MIN);
    expect(nearest([], 1)).toBeNull();
  });
  it('rounds an axis top up', () => {
    expect(niceMax(0)).toBe(1);
    expect(niceMax(3.2)).toBe(5);
    expect(niceMax(1.8)).toBe(2);
    expect(niceMax(22)).toBe(25);
    expect(niceMax(100)).toBe(100);
    expect(niceBytes(3.7 * 1024 ** 3)).toBe(4 * 1024 ** 3);
    expect(niceBytes(300 * 1024 ** 2)).toBe(400 * 1024 ** 2);
  });
});

describe('approvalsAsked', () => {
  it('counts each approval once from events and the store, inside the range', () => {
    const ev = (seq: number, ts: number, session_id: string, approval_id?: string): EventRecord => ({
      seq,
      ts,
      session_id,
      project_id: null,
      kind: 'approval.requested',
      actor: 'agent',
      payload: approval_id ? { approval_id } : {},
    });
    const events = [ev(1, 10, 'a', 'x1'), ev(2, 20, 'a', 'x2'), ev(3, 1, 'a', 'old'), ev(4, 30, 'b'), { ...ev(5, 30, 'a', 'x3'), kind: 'approval.decided' }];
    const approvals = [{ id: 'x2', session_id: 'a', created_at: 20 }, { id: 'x9', session_id: 'b', created_at: 40 }] as Approval[];
    const m = approvalsAsked(events, approvals, 5);
    expect(m.get('a')).toBe(2);
    expect(m.get('b')).toBe(2);
  });
});

describe('rows', () => {
  const now = 200 * MIN;
  const s = session('a', {
    status: 'idle',
    status_since: now - 3 * 3_600_000,
    history: [
      { status: 'working', at: now - 50 * MIN },
      { status: 'needs_you', at: now - 30 * MIN },
      { status: 'idle', at: now - 15 * MIN },
    ],
    usage: { cost_usd: 2, context_used_percentage: 75, context_window_size: 200000, lines_added: 5, lines_removed: 3, model: 'Opus', effort: null, updated_at: 0 },
  });
  const stats: Stats = {
    db_bytes: 0,
    data_bytes: 0,
    sessions: [{ session_id: 'a', memory_bytes: 2 * 1024 ** 3, processes: 2, scrollback_bytes: 0, transcript_bytes: 100, folder_bytes: null }],
  };
  it('builds a row with times, disk and rules', () => {
    const [row] = buildRows({ sessions: [s], projects, stats, asked: new Map([['a', 3]]), costs: new Map(), now, rangeMs: 3_600_000 });
    expect(row.projectName).toBe('atlas');
    expect(row.workedMs).toBe(20 * MIN);
    expect(row.waitedMs).toBe(15 * MIN);
    expect(row.disk).toBe(100);
    expect(row.asked).toBe(3);
    expect(row.recs.map((r) => r.id)).toEqual(['compact', 'memory', 'stop', 'waits']);
    expect(compactTargets([row]).map((r) => r.id)).toEqual(['a']);
    expect(compactTargets([{ ...row, status: 'working' }])).toEqual([]);
  });
  it('sorts by a column with empty values last', () => {
    const r = (id: string, cost: number | null) => ({ id, title: id, cost, linesAdded: null, linesRemoved: null }) as SessionRow;
    const rows = [r('a', 1), r('b', null), r('c', 3)];
    expect(sortRows(rows, 'cost', 'desc').map((x) => x.id)).toEqual(['c', 'a', 'b']);
    expect(sortRows(rows, 'cost', 'asc').map((x) => x.id)).toEqual(['a', 'c', 'b']);
  });
});

describe('xTicks', () => {
  it('puts ticks on round local times inside the range', () => {
    const since = new Date(2026, 10, 24, 9, 3).getTime();
    const hour = xTicks(since, since + 3_600_000, '1h').map((t) => new Date(t).getMinutes());
    expect(hour).toEqual([10, 20, 30, 40, 50, 0]);
    const day = xTicks(since, since + 24 * 3_600_000, '24h').map((t) => new Date(t).getHours());
    expect(day).toEqual([12, 16, 20, 0, 4, 8]);
    const week = xTicks(since, since + 7 * 24 * 3_600_000, '7d');
    expect(week).toHaveLength(7);
    expect(week.every((t) => new Date(t).getHours() === 0)).toBe(true);
  });
});

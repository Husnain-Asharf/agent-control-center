import { describe, expect, it } from 'vitest';
import { canCompact, planHigh, recommend, type RecInput } from './recommend';

const base: RecInput = { status: 'idle', contextPct: 10, memoryBytes: 100 * 1024 ** 2, idleMs: 0, waitedMs: 0 };
const ids = (i: Partial<RecInput>) => recommend({ ...base, ...i }).map((r) => r.id);

describe('recommend', () => {
  it('gives nothing for a calm session', () => {
    expect(ids({})).toEqual([]);
  });
  it('asks to compact from 70% context', () => {
    expect(ids({ contextPct: 69.9 })).toEqual([]);
    expect(ids({ contextPct: 70 })).toEqual(['compact']);
    expect(ids({ contextPct: null })).toEqual([]);
  });
  it('flags memory above 1.5 GB', () => {
    expect(ids({ memoryBytes: 1.5 * 1024 ** 3 })).toEqual([]);
    expect(ids({ memoryBytes: 1.6 * 1024 ** 3 })).toEqual(['memory']);
    expect(ids({ memoryBytes: null })).toEqual([]);
  });
  it('suggests stopping after 2 hours idle', () => {
    expect(ids({ idleMs: 2 * 3_600_000 })).toEqual([]);
    expect(ids({ idleMs: 2 * 3_600_000 + 1 })).toEqual(['stop']);
    expect(ids({ status: 'working', idleMs: 3 * 3_600_000 })).toEqual([]);
  });
  it('notes a session that waited more than 10 minutes', () => {
    expect(ids({ waitedMs: 10 * 60_000 })).toEqual([]);
    expect(ids({ waitedMs: 11 * 60_000, status: 'needs_you' })).toEqual(['waits']);
  });
  it('gives nothing for a session that is not running', () => {
    expect(ids({ status: 'done', contextPct: 90, memoryBytes: 2 * 1024 ** 3, waitedMs: 3_600_000 })).toEqual([]);
    expect(ids({ status: 'stopped', contextPct: 90 })).toEqual([]);
  });
  it('lists several in a fixed order', () => {
    expect(ids({ contextPct: 80, memoryBytes: 2 * 1024 ** 3, idleMs: 3 * 3_600_000, waitedMs: 20 * 60_000 })).toEqual([
      'compact',
      'memory',
      'stop',
      'waits',
    ]);
  });
});

describe('planHigh', () => {
  it('is true from 85% of the 5-hour window', () => {
    expect(planHigh(null)).toBe(false);
    expect(planHigh({ five_hour: null, seven_day: { used_percentage: 99, resets_at: null }, updated_at: 0 })).toBe(false);
    expect(planHigh({ five_hour: { used_percentage: 84.9, resets_at: null }, seven_day: null, updated_at: 0 })).toBe(false);
    expect(planHigh({ five_hour: { used_percentage: 85, resets_at: null }, seven_day: null, updated_at: 0 })).toBe(true);
  });
});

describe('canCompact', () => {
  it('allows only an idle session', () => {
    expect(canCompact('idle')).toEqual({ ok: true, why: null });
    expect(canCompact('working').ok).toBe(false);
    expect(canCompact('starting').ok).toBe(false);
    expect(canCompact('needs_you').why).toMatch(/Waiting/);
    expect(canCompact('done').why).toBe('Not running.');
  });
});

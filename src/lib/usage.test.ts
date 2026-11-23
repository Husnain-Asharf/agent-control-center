import { describe, expect, it } from 'vitest';
import { fmtCost, level, resetText, usageByProject, usageLine } from './usage';
import type { Session } from './types';

const usage = (cost: number, ctx: number | null = null) => ({
  cost_usd: cost,
  context_used_percentage: ctx,
  context_window_size: 200000,
  lines_added: 0,
  lines_removed: 0,
  model: null,
  effort: null,
  updated_at: 1,
});
const session = (id: string, project: string, u: ReturnType<typeof usage> | null) => ({ id, project_id: project, usage: u }) as unknown as Session;

describe('usage', () => {
  it('formats cost', () => {
    expect(fmtCost(2.345)).toBe('$2.35');
    expect(fmtCost(0.4)).toBe('$0.40');
    expect(fmtCost(Number.NaN)).toBe('$0.00');
  });
  it('grades a share of a limit', () => {
    expect(level(null)).toBe('ok');
    expect(level(59)).toBe('ok');
    expect(level(60)).toBe('warn');
    expect(level(85)).toBe('high');
  });
  it('says when a window resets', () => {
    const now = 1_000_000_000_000;
    const at = (min: number) => (now + min * 60_000) / 1000;
    expect(resetText(null, now)).toBeNull();
    expect(resetText(at(-5), now)).toBe('now');
    expect(resetText(at(42), now)).toBe('in 42 min');
    expect(resetText(at(130), now)).toBe('in 2 h 10 min');
    expect(resetText(at(120), now)).toBe('in 2 h');
    expect(resetText(at(60 * 24 * 3), now)).toBe('in 3 d');
  });
  it('totals cost by project, highest first, and skips sessions with no report', () => {
    const rows = usageByProject([
      session('a', 'p1', usage(1)),
      session('b', 'p2', usage(5)),
      session('c', 'p1', usage(2.5)),
      session('d', 'p3', null),
    ]);
    expect(rows).toEqual([
      { projectId: 'p2', costUsd: 5, sessions: 1 },
      { projectId: 'p1', costUsd: 3.5, sessions: 2 },
    ]);
  });
  it('writes the row line', () => {
    expect(usageLine({ usage: null })).toBeNull();
    expect(usageLine({ usage: usage(2.345, 64.4) })).toBe('$2.35 · context 64%');
    expect(usageLine({ usage: usage(1) })).toBe('$1.00');
  });
});

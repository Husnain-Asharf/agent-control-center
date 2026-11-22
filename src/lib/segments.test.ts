import { describe, expect, it } from 'vitest';
import { marksFor, segmentsFor } from './segments';
import type { Session, SessionStatus } from './types';

const MIN = 60_000;
const NOW = 1_000 * MIN;

const h = (...e: [SessionStatus, number][]) => e.map(([status, at]) => ({ status, at }));

describe('segmentsFor', () => {
  it('segments clip to window', () => {
    const history = h(['working', NOW - 90 * MIN], ['needs_you', NOW - 30 * MIN], ['working', NOW - 10 * MIN]);
    const segs = segmentsFor(history, NOW, 60 * MIN);
    expect(segs).toEqual([
      { kind: 'work', start: NOW - 60 * MIN, end: NOW - 30 * MIN },
      { kind: 'needs', start: NOW - 30 * MIN, end: NOW - 10 * MIN },
      { kind: 'work', start: NOW - 10 * MIN, end: null },
    ]);
  });

  it('drops entries that ended before the window', () => {
    const history = h(['working', NOW - 200 * MIN], ['idle', NOW - 90 * MIN], ['working', NOW - 5 * MIN]);
    const segs = segmentsFor(history, NOW, 60 * MIN);
    expect(segs).toEqual([
      { kind: 'idle', start: NOW - 60 * MIN, end: NOW - 5 * MIN },
      { kind: 'work', start: NOW - 5 * MIN, end: null },
    ]);
  });

  it('open last segment has null end', () => {
    const segs = segmentsFor(h(['starting', NOW - 3 * MIN]), NOW, 15 * MIN);
    expect(segs).toEqual([{ kind: 'work', start: NOW - 3 * MIN, end: null }]);
  });

  it('needs_you maps to needs and idle to idle', () => {
    const segs = segmentsFor(h(['working', NOW - 9 * MIN], ['needs_you', NOW - 6 * MIN], ['idle', NOW - 2 * MIN]), NOW, 15 * MIN);
    expect(segs.map((s) => s.kind)).toEqual(['work', 'needs', 'idle']);
  });

  it('terminal states produce marks not segments', () => {
    const segs = segmentsFor(h(['working', NOW - 9 * MIN], ['done', NOW - 4 * MIN]), NOW, 15 * MIN);
    expect(segs).toEqual([{ kind: 'work', start: NOW - 9 * MIN, end: NOW - 4 * MIN }]);
    expect(segmentsFor(h(['working', NOW - 9 * MIN], ['failed', NOW - 4 * MIN], ['stopped', NOW - 3 * MIN]), NOW, 15 * MIN)).toHaveLength(1);
  });
});

describe('marksFor', () => {
  const base = { ended_at: null, status_since: NOW - MIN } as Session;
  it('marks done and failed sessions', () => {
    expect(marksFor({ ...base, status: 'done', ended_at: NOW - 2 * MIN })).toEqual({ kind: 'done', at: NOW - 2 * MIN });
    expect(marksFor({ ...base, status: 'failed' })).toEqual({ kind: 'failed', at: NOW - MIN });
  });
  it('has no mark otherwise', () => {
    expect(marksFor({ ...base, status: 'working' })).toBeNull();
    expect(marksFor({ ...base, status: 'stopped' })).toBeNull();
  });
});

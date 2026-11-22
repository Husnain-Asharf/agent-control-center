import { describe, expect, it } from 'vitest';
import { createReplayQueue } from './snapshotQueue';

describe('createReplayQueue', () => {
  it('does not queue while idle', () => {
    const q = createReplayQueue<number>();
    expect(q.push(1)).toBe(false);
    expect(q.active()).toBe(false);
  });

  it('queues events in order while a snapshot is in flight and hands them back at the end', () => {
    const q = createReplayQueue<number>();
    const t = q.begin();
    expect(q.active()).toBe(true);
    expect(q.push(1)).toBe(true);
    expect(q.push(2)).toBe(true);
    expect(q.end(t)).toEqual([1, 2]);
    expect(q.active()).toBe(false);
    expect(q.push(3)).toBe(false);
  });

  it('a newer begin supersedes the older one, which then returns nothing', () => {
    const q = createReplayQueue<number>();
    const t1 = q.begin();
    q.push(1);
    const t2 = q.begin();
    q.push(2);
    expect(q.end(t1)).toEqual([]);
    expect(q.active()).toBe(true);
    // Events held for the older request are covered by the newer snapshot.
    expect(q.end(t2)).toEqual([2]);
  });

  it('end is safe to call twice', () => {
    const q = createReplayQueue<number>();
    const t = q.begin();
    q.push(1);
    expect(q.end(t)).toEqual([1]);
    expect(q.end(t)).toEqual([]);
  });
});

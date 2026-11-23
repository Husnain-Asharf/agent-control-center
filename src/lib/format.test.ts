import { describe, expect, it } from 'vitest';
import { clock, fmtDur, fmtTimer, folderLabel, monogram, worktreeLabel } from './format';

describe('monogram', () => {
  it('uses the first letters of the first two words', () => {
    expect(monogram('atlas-api')).toBe('AA');
    expect(monogram('payments_gateway_v2')).toBe('PG');
    expect(monogram('my app')).toBe('MA');
  });
  it('uses the first two letters of a single word', () => {
    expect(monogram('atlas')).toBe('AT');
    expect(monogram('x')).toBe('X');
  });
  it('gives ? for an empty name', () => {
    expect(monogram('')).toBe('?');
    expect(monogram(' - ')).toBe('?');
  });
});

describe('fmtDur', () => {
  it('formats seconds, minutes and hours', () => {
    expect(fmtDur(42_000)).toBe('42s');
    expect(fmtDur(59_999)).toBe('59s');
    expect(fmtDur(12 * 60_000)).toBe('12m');
    expect(fmtDur(65 * 60_000)).toBe('1h 05m');
  });
  it('clamps negative values to 0s', () => {
    expect(fmtDur(-5)).toBe('0s');
  });
});

describe('fmtTimer', () => {
  it('formats m:ss', () => {
    expect(fmtTimer(252_000)).toBe('4:12');
    expect(fmtTimer(5_000)).toBe('0:05');
    expect(fmtTimer(-1)).toBe('0:00');
  });
  it('formats H:MM:SS from one hour', () => {
    expect(fmtTimer(3_600_000)).toBe('1:00:00');
    expect(fmtTimer(3_725_000)).toBe('1:02:05');
  });
});

describe('clock', () => {
  it('formats local time as HH:MM', () => {
    expect(clock(new Date(2026, 0, 1, 9, 5).getTime())).toBe('09:05');
    expect(clock(new Date(2026, 0, 1, 23, 40).getTime())).toBe('23:40');
  });
});

describe('folderLabel', () => {
  it('says project folder for a session there, and names its worktree otherwise', () => {
    expect(folderLabel({ mode: 'project', worktree: '/Users/x/app' })).toBe('Project folder');
    expect(folderLabel({ mode: 'worktree', worktree: '/Users/x/app/.worktrees/fix' })).toBe('.worktrees/fix');
    expect(folderLabel({ worktree: '/Users/x/app/.worktrees/fix' })).toBe('.worktrees/fix');
  });
});

describe('worktreeLabel', () => {
  it('names the worktree folder', () => {
    expect(worktreeLabel('/Users/x/my app/.worktrees/fix-the-bug')).toBe('.worktrees/fix-the-bug');
    expect(worktreeLabel('/Users/x/app/.worktrees/fix/')).toBe('.worktrees/fix');
  });
  it('reads Windows paths, with either slash', () => {
    expect(worktreeLabel('C:/Users/x/my app\\.worktrees\\fix-the-bug')).toBe('.worktrees/fix-the-bug');
    expect(worktreeLabel('C:\\Users\\x\\app\\.worktrees\\fix')).toBe('.worktrees/fix');
  });
});

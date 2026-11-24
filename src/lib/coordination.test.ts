import { describe, expect, it } from 'vitest';
import { coordination, fromFolder } from './coordination';
import type { EventRecord, Session } from './types';

const s = (id: string, mode: 'project' | 'worktree', worktree = '/repo') => ({ id, mode, worktree }) as unknown as Session;
let seq = 0;
const used = (session: string, tool: string, summary: string, ts = 100): EventRecord => ({
  seq: ++seq,
  ts,
  session_id: session,
  project_id: 'p',
  kind: 'tool.used',
  actor: 'agent',
  payload: { tool, summary },
});

describe('coordination', () => {
  it('shows paths from the folder', () => {
    expect(fromFolder('/repo/src/a.ts', '/repo')).toBe('src/a.ts');
    expect(fromFolder('C:\\repo\\src\\a.ts', 'C:\\repo\\')).toBe('src/a.ts');
    expect(fromFolder('/elsewhere/a.ts', '/repo')).toBe('/elsewhere/a.ts');
  });
  it('lists each session’s changed files and the files two sessions both changed', () => {
    const c = coordination(
      [s('a', 'project'), s('b', 'project'), s('c', 'worktree', '/repo/.worktrees/c')],
      [
        used('a', 'Edit', '/repo/src/api.ts', 110),
        used('a', 'Write', '/repo/src/shared.ts', 120),
        used('b', 'Edit', '/repo/src/shared.ts', 130),
        used('b', 'Read', '/repo/src/api.ts', 140),
        used('b', 'mcp__acc__send_message', 'mcp__acc__send_message', 150),
        used('c', 'Edit', '/repo/.worktrees/c/src/x.ts', 160),
        used('a', 'Edit', '/repo/old.ts', 10),
      ],
      100,
    );
    expect(c.shared.map((w) => [w.sessionId, w.files, w.messages])).toEqual([
      ['a', ['src/shared.ts', 'src/api.ts'], 0],
      ['b', ['src/shared.ts'], 1],
    ]);
    expect(c.separate).toEqual([{ sessionId: 'c', files: ['src/x.ts'], messages: 0 }]);
    expect(c.overlaps).toEqual([{ file: 'src/shared.ts', sessionIds: ['a', 'b'] }]);
  });
});

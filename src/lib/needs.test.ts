import { describe, expect, it } from 'vitest';
import { needsYouForProject } from './needs';
import type { Approval, Session } from './types';

const session = (id: string, project_id: string, status: Session['status'] = 'working'): Session => ({
  id,
  project_id,
  agent: 'claude',
  title: id,
  branch: `agent/${id}`,
  worktree: `/tmp/${id}`,
  status,
  status_since: 0,
  started_at: 0,
  ended_at: null,
  exit_code: null,
  history: [],
});

const approval = (id: string, session_id: string, state: Approval['state'] = 'pending'): Approval => ({
  id,
  session_id,
  tool: 'Bash',
  summary: 'ls',
  input: null,
  created_at: 0,
  state,
  decided_by: null,
});

const byId = <T extends { id: string }>(l: T[]) => Object.fromEntries(l.map((x) => [x.id, x]));

describe('needsYouForProject', () => {
  const sessions = byId([session('a', 'p1'), session('b', 'p1', 'failed'), session('c', 'p2', 'failed'), session('d', 'p2')]);

  it('counts a pending approval in the project', () => {
    expect(needsYouForProject('p1', byId([session('a', 'p1')]), byId([approval('x', 'a')]))).toBe(1);
  });
  it('counts a failed session in the project', () => {
    expect(needsYouForProject('p1', sessions, {})).toBe(1);
  });
  it('ignores approvals and failures from other projects', () => {
    expect(needsYouForProject('p1', sessions, byId([approval('x', 'd')]))).toBe(1);
    expect(needsYouForProject('p3', sessions, byId([approval('x', 'd')]))).toBe(0);
  });
  it('ignores resolved approvals', () => {
    expect(needsYouForProject('p1', sessions, byId([approval('x', 'a', 'allowed')]))).toBe(1);
  });
  it('adds both', () => {
    expect(needsYouForProject('p1', sessions, byId([approval('x', 'a')]))).toBe(2);
  });
});

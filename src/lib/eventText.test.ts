import { describe, expect, it } from 'vitest';
import { eventText } from './eventText';

const ev = (kind: string, payload: unknown, actor = 'agent') => eventText({ kind, actor, payload });

describe('eventText', () => {
  it('project.added', () => {
    expect(ev('project.added', { name: 'api', path: '/x/api' }, 'user')).toBe('Added project api');
  });
  it('session.started and resumed', () => {
    const base = { title: 't', agent: 'claude', branch: 'agent/fix', worktree: '/w' };
    expect(ev('session.started', { ...base, resume: false })).toBe('Started on branch agent/fix');
    expect(ev('session.started', { ...base, resume: true })).toBe('Resumed on branch agent/fix');
  });
  it('session.status uses the status label', () => {
    expect(ev('session.status', { status: 'needs_you' })).toBe('Now Needs you');
    expect(ev('session.status', { status: 'working' })).toBe('Now Working');
  });
  it('session.exited', () => {
    expect(ev('session.exited', { exit_code: 0, status: 'done' })).toBe('Exited with code 0');
    expect(ev('session.exited', { exit_code: null, status: 'failed' })).toBe('Ended without an exit code (Failed)');
  });
  it('approval.requested', () => {
    expect(ev('approval.requested', { approval_id: 'a', tool: 'Bash', summary: 'npm test' })).toBe('Asked to use Bash: npm test');
  });
  it('approval.decided by the user', () => {
    const p = { approval_id: 'a', tool: 'Bash', summary: 'npm test' };
    expect(ev('approval.decided', { ...p, decision: 'allow_once', state: 'allowed', rule: null }, 'user')).toBe('Allowed once: Bash: npm test');
    expect(ev('approval.decided', { ...p, decision: 'deny', state: 'denied', rule: null }, 'user')).toBe('Denied: Bash: npm test');
    expect(
      ev('approval.decided', { ...p, decision: 'allow_always', state: 'allowed', rule: { tool: 'Bash', prefix: 'npm test' } }, 'user'),
    ).toBe('Always allowed npm test: Bash: npm test');
  });
  it('approval.decided by a rule', () => {
    expect(
      ev('approval.decided', { approval_id: 'a', tool: 'Edit', summary: 'Edit', state: 'allowed', rule: { tool: 'Edit', prefix: null } }, 'policy'),
    ).toBe('Allowed by a saved rule: Edit');
  });
  it('approval.decided with a rule shows what the rule covers', () => {
    const p = { approval_id: 'a', tool: 'Edit', summary: 'src/a.ts', decision: 'allow_always', state: 'allowed' };
    expect(ev('approval.decided', { ...p, rule: { tool: 'Edit', prefix: null } }, 'user')).toBe('Always allowed Edit requests: Edit: src/a.ts');
    expect(ev('approval.decided', { approval_id: 'a', tool: 'Bash', summary: 'npm test', state: 'allowed', rule: { tool: 'Bash', prefix: 'npm test' } }, 'policy')).toBe(
      'Allowed by a saved rule (npm test): Bash: npm test',
    );
  });
  it('approval.decided with once_only gives the reason and no rule', () => {
    const p = { approval_id: 'a', tool: 'Bash', summary: 'bash -c x', decision: 'allow_always', state: 'allowed', rule: null };
    expect(ev('approval.decided', { ...p, once_only: 'runs inline code' }, 'user')).toBe('Allowed once, no rule added (runs inline code): Bash: bash -c x');
  });
  it('approval.cancelled and expired', () => {
    const p = { approval_id: 'a', tool: 'Bash', summary: 'ls' };
    expect(ev('approval.cancelled', p)).toBe('Request cancelled: Bash: ls');
    expect(ev('approval.expired', p, 'system')).toBe('Request expired: Bash: ls');
  });
  it('tool.used', () => {
    expect(ev('tool.used', { tool: 'Read', summary: 'src/a.ts' })).toBe('Used Read: src/a.ts');
    expect(ev('tool.used', { tool: 'Task', summary: 'Task' })).toBe('Used Task');
  });
  it('never throws on odd payloads', () => {
    expect(ev('tool.used', null)).toBe('Used a tool');
    expect(ev('session.status', 'x')).toBe('Now unknown');
    expect(ev('approval.decided', [], 'user')).toBe('Decided: a tool');
    expect(ev('something.else', {})).toBe('something.else');
  });
});

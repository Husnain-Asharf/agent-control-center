import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { needsYouCount, oldestPending, pendingForSession, sessionsByProject, useStore } from './store';
import type { Approval, EventRecord, Project, Session } from '../lib/types';

const project = (id: string): Project => ({
  id,
  name: `proj-${id}`,
  path: `/p/${id}`,
  host: null,
  base_branch: 'main',
  added_at: 1,
});

const session = (id: string, project_id = 'p1', status: Session['status'] = 'working'): Session => ({
  id,
  project_id,
  agent: 'claude',
  title: `task ${id}`,
  branch: `agent/${id}`,
  worktree: `/w/${id}`,
  status,
  status_since: 1,
  started_at: 1,
  ended_at: null,
  exit_code: null,
  history: [],
});

const approval = (id: string, state: Approval['state'] = 'pending'): Approval => ({
  id,
  session_id: 's1',
  tool: 'Bash',
  summary: 'npm install',
  input: null,
  created_at: 1,
  state,
  decided_by: null,
});

const record = (seq: number): EventRecord => ({
  seq,
  ts: seq,
  session_id: null,
  project_id: null,
  kind: 'tool.used',
  actor: 'agent',
  payload: null,
});

beforeEach(() => {
  useStore.setState(useStore.getInitialState(), true);
});

describe('store', () => {
  it('applySnapshot replaces everything', () => {
    const { applySnapshot, applyEvent } = useStore.getState();
    applyEvent({ kind: 'session_updated', data: session('old') });
    applySnapshot({
      projects: [project('p1')],
      sessions: [session('s1')],
      approvals: [approval('a1')],
    });
    const s = useStore.getState();
    expect(Object.keys(s.projects)).toEqual(['p1']);
    expect(Object.keys(s.sessions)).toEqual(['s1']);
    expect(Object.keys(s.approvals)).toEqual(['a1']);
  });

  it('session_updated upserts by id', () => {
    const { applyEvent } = useStore.getState();
    applyEvent({ kind: 'session_updated', data: session('s1', 'p1', 'working') });
    applyEvent({ kind: 'session_updated', data: session('s1', 'p1', 'needs_you') });
    const s = useStore.getState();
    expect(Object.keys(s.sessions)).toEqual(['s1']);
    expect(s.sessions.s1.status).toBe('needs_you');
  });

  it('approval_created adds pending', () => {
    useStore.getState().applyEvent({ kind: 'approval_created', data: approval('a1') });
    expect(useStore.getState().approvals.a1.state).toBe('pending');
  });

  it('approval_resolved removes from pending', () => {
    const { applyEvent } = useStore.getState();
    applyEvent({ kind: 'approval_created', data: approval('a1') });
    applyEvent({ kind: 'approval_resolved', data: approval('a1', 'allowed') });
    expect(useStore.getState().approvals.a1).toBeUndefined();
  });

  it('event_appended keeps newest 500', () => {
    const { applyEvent } = useStore.getState();
    for (let i = 1; i <= 510; i++) applyEvent({ kind: 'event_appended', data: record(i) });
    const { events } = useStore.getState();
    expect(events).toHaveLength(500);
    expect(events[0].seq).toBe(510);
    expect(events[499].seq).toBe(11);
  });

  it('needsYouCount counts pending approvals and failed sessions', () => {
    const { applySnapshot } = useStore.getState();
    applySnapshot({
      projects: [project('p1')],
      sessions: [session('s1', 'p1', 'failed'), session('s2', 'p1', 'working')],
      approvals: [approval('a1'), approval('a2')],
    });
    expect(needsYouCount(useStore.getState())).toBe(3);
  });

  it('applyEvent ignores session_output', () => {
    const before = useStore.getState();
    before.applyEvent({ kind: 'session_output', data: { session_id: 's1', data_b64: 'aGk=', offset: 0 } });
    expect(useStore.getState()).toBe(before);
  });

  it('sessionsByProject groups sessions under their project', () => {
    useStore.getState().applySnapshot({
      projects: [project('p1'), project('p2')],
      sessions: [session('s1', 'p1'), session('s2', 'p2'), session('s3', 'p1')],
      approvals: [],
    });
    const groups = sessionsByProject(useStore.getState());
    expect(groups.map((g) => [g.project.id, g.sessions.length])).toEqual([
      ['p1', 2],
      ['p2', 1],
    ]);
  });
});

describe('persisted theme', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  it('applies a saved light theme to the document at startup', async () => {
    const root = { dataset: {} as Record<string, string> };
    vi.stubGlobal('document', { documentElement: root });
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => (k === 'acc-theme' ? 'light' : null),
      setItem: () => {},
    });
    vi.resetModules();
    const fresh = await import('./store');
    expect(fresh.useStore.getState().theme).toBe('light');
    expect(root.dataset.theme).toBe('light');
  });
});

describe('sound setting', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  const load = async (saved: string | null) => {
    const writes: Record<string, string> = {};
    vi.stubGlobal('document', { documentElement: { dataset: {} } });
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => (k === 'acc-sound' ? saved : null),
      setItem: (k: string, v: string) => {
        writes[k] = v;
      },
    });
    vi.resetModules();
    return { fresh: await import('./store'), writes };
  };

  it('defaults to on', async () => {
    const { fresh } = await load(null);
    expect(fresh.useStore.getState().soundEnabled).toBe(true);
  });

  it('reads a saved off value', async () => {
    const { fresh } = await load('off');
    expect(fresh.useStore.getState().soundEnabled).toBe(false);
  });

  it('toggles and persists', async () => {
    const { fresh, writes } = await load(null);
    fresh.useStore.getState().toggleSound();
    expect(fresh.useStore.getState().soundEnabled).toBe(false);
    expect(writes['acc-sound']).toBe('off');
    fresh.useStore.getState().toggleSound();
    expect(fresh.useStore.getState().soundEnabled).toBe(true);
    expect(writes['acc-sound']).toBe('on');
  });
});

describe('text settings', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  const load = async (saved: Record<string, string> | null) => {
    const writes: Record<string, string> = {};
    const root = { dataset: {} as Record<string, string> };
    vi.stubGlobal('document', { documentElement: root });
    vi.stubGlobal(
      'localStorage',
      saved && {
        getItem: (k: string) => saved[k] ?? null,
        setItem: (k: string, v: string) => {
          writes[k] = v;
        },
      },
    );
    vi.resetModules();
    return { fresh: await import('./store'), writes, root };
  };

  it('reads saved values and marks <html> for bolder text', async () => {
    const { fresh, root } = await load({ 'acc-text-scale': '1.25', 'acc-term-font-size': '14', 'acc-bolder': 'on' });
    const s = fresh.useStore.getState();
    expect([s.textScale, s.termFontSize, s.bolder]).toEqual([1.25, 14, true]);
    expect('bolder' in root.dataset).toBe(true);
  });

  it('falls back to the defaults for invalid or old values', async () => {
    const { fresh, root } = await load({ 'acc-text-scale': '1.5', 'acc-term-font-size': 'big', 'acc-bolder': 'yes' });
    const s = fresh.useStore.getState();
    expect([s.textScale, s.termFontSize, s.bolder]).toEqual([1, 12.5, false]);
    expect('bolder' in root.dataset).toBe(false);
  });

  it('uses the defaults without storage', async () => {
    const { fresh } = await load(null);
    const s = fresh.useStore.getState();
    expect([s.textScale, s.termFontSize, s.bolder]).toEqual([1, 12.5, false]);
  });

  it('saves changes', async () => {
    const { fresh, writes, root } = await load({});
    const { setTextScale, setTermFontSize, toggleBolder } = fresh.useStore.getState();
    setTextScale(1.1);
    setTermFontSize(16);
    toggleBolder();
    const s = fresh.useStore.getState();
    expect([s.textScale, s.termFontSize, s.bolder]).toEqual([1.1, 16, true]);
    expect(writes).toEqual({ 'acc-text-scale': '1.1', 'acc-term-font-size': '16', 'acc-bolder': 'on' });
    expect('bolder' in root.dataset).toBe(true);
    fresh.useStore.getState().toggleBolder();
    expect(writes['acc-bolder']).toBe('off');
    expect('bolder' in root.dataset).toBe(false);
  });
});

describe('glass settings', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  const load = async (saved: Record<string, string>) => {
    const writes: Record<string, string> = {};
    vi.stubGlobal('document', { documentElement: { dataset: {} } });
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => saved[k] ?? null,
      setItem: (k: string, v: string) => {
        writes[k] = v;
      },
    });
    vi.resetModules();
    return { fresh: await import('./store'), writes };
  };

  it('reads saved blur and opacity', async () => {
    const { fresh } = await load({ 'acc-glass-blur': '24', 'acc-glass-opacity': '45' });
    const s = fresh.useStore.getState();
    expect([s.glassBlur, s.glassOpacity]).toEqual([24, 45]);
  });

  it('defaults to 16px and Auto', async () => {
    const { fresh } = await load({});
    const s = fresh.useStore.getState();
    expect([s.glassBlur, s.glassOpacity]).toEqual([16, null]);
  });

  it('saves changes, and Auto as auto', async () => {
    const { fresh, writes } = await load({});
    fresh.useStore.getState().setGlassBlur(30);
    fresh.useStore.getState().setGlassOpacity(70);
    expect(writes).toEqual({ 'acc-glass-blur': '30', 'acc-glass-opacity': '70' });
    fresh.useStore.getState().setGlassOpacity(null);
    expect(writes['acc-glass-opacity']).toBe('auto');
    expect(fresh.useStore.getState().glassOpacity).toBeNull();
  });
});

describe('colors', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  const load = async (saved: Record<string, string>) => {
    const props = new Map<string, string>();
    const style = {
      setProperty: (k: string, v: string) => props.set(k, v),
      removeProperty: (k: string) => props.delete(k),
    };
    vi.stubGlobal('document', { documentElement: { dataset: {}, style } });
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => saved[k] ?? null,
      setItem: (k: string, v: string) => {
        saved[k] = v;
      },
      removeItem: (k: string) => {
        delete saved[k];
      },
    });
    vi.resetModules();
    return { fresh: await import('./store'), props };
  };

  it('applies the saved colors of the current theme at startup and on a theme switch', async () => {
    const warm = JSON.stringify({ preset: 'warm', colors: {} });
    const { fresh, props } = await load({ 'acc-colors-dark': warm });
    expect(props.get('--panel')).toBe('#1a1814');
    fresh.useStore.getState().toggleTheme();
    expect(props.size).toBe(0);
    fresh.useStore.getState().toggleTheme();
    expect(props.get('--panel')).toBe('#1a1814');
  });

  it('saves changes per theme and reset clears them', async () => {
    const saved: Record<string, string> = {};
    const { fresh, props } = await load(saved);
    const st = () => fresh.useStore.getState();
    st().setColor('cyan', '#0088FF');
    expect(props.get('--cyan')).toBe('#0088ff');
    expect(JSON.parse(saved['acc-colors-dark']).colors.cyan).toBe('#0088ff');
    expect(saved['acc-colors-light']).toBeUndefined();
    st().setColorPreset('blue');
    expect(JSON.parse(saved['acc-colors-dark']).preset).toBe('blue');
    st().resetColors();
    expect(saved['acc-colors-dark']).toBeUndefined();
    expect(props.size).toBe(0);
  });
});

describe('connEpoch', () => {
  it('increments on false to true and not on true to true', () => {
    const { setConnected } = useStore.getState();
    setConnected(false);
    const base = useStore.getState().connEpoch;
    setConnected(true);
    expect(useStore.getState().connEpoch).toBe(base + 1);
    setConnected(true);
    expect(useStore.getState().connEpoch).toBe(base + 1);
    setConnected(false);
    expect(useStore.getState().connEpoch).toBe(base + 1);
    setConnected(true);
    expect(useStore.getState().connEpoch).toBe(base + 2);
  });
});

describe('approval selectors', () => {
  const ap = (id: string, session_id: string, created_at: number): Approval => ({
    ...approval(id),
    session_id,
    created_at,
  });

  it('oldestPending picks earliest created_at', () => {
    const { applyEvent } = useStore.getState();
    applyEvent({ kind: 'approval_created', data: ap('late', 's1', 300) });
    applyEvent({ kind: 'approval_created', data: ap('early', 's2', 100) });
    applyEvent({ kind: 'approval_created', data: ap('mid', 's1', 200) });
    expect(oldestPending(useStore.getState())?.id).toBe('early');
  });

  it('oldestPending is undefined with nothing pending', () => {
    expect(oldestPending(useStore.getState())).toBeUndefined();
  });

  it('pendingForSession filters by session', () => {
    const { applyEvent } = useStore.getState();
    applyEvent({ kind: 'approval_created', data: ap('a', 's1', 300) });
    applyEvent({ kind: 'approval_created', data: ap('b', 's2', 100) });
    applyEvent({ kind: 'approval_created', data: ap('c', 's1', 200) });
    expect(pendingForSession(useStore.getState(), 's1')?.id).toBe('c');
    expect(pendingForSession(useStore.getState(), 's2')?.id).toBe('b');
    expect(pendingForSession(useStore.getState(), 's3')).toBeUndefined();
  });
});

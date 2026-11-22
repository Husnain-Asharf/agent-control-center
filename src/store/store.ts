import { create } from 'zustand';
import type {
  Approval,
  DaemonEvent,
  EventRecord,
  Project,
  Session,
  Snapshot,
  Theme,
  Toast,
} from '../lib/types';

export const MAX_EVENTS = 500;
const THEME_KEY = 'acc-theme';
const SOUND_KEY = 'acc-sound';

export interface State {
  connected: boolean;
  /** The bridge's last connect error, shown while disconnected. */
  daemonError: string | null;
  /** Bumped each time the daemon connection comes back. Views that stream keyed on it remount. */
  connEpoch: number;
  projects: Record<string, Project>;
  sessions: Record<string, Session>;
  /** Pending approvals only. Resolved ones leave the store. */
  approvals: Record<string, Approval>;
  /** Newest first, at most MAX_EVENTS. */
  events: EventRecord[];
  toasts: Toast[];
  theme: Theme;
  soundEnabled: boolean;
  applySnapshot: (s: Snapshot) => void;
  applyEvent: (e: DaemonEvent) => void;
  setConnected: (b: boolean, error?: string | null) => void;
  pushToast: (t: Omit<Toast, 'id' | 'kind'> & { kind?: Toast['kind'] }) => void;
  dismissToast: (id: number) => void;
  toggleTheme: () => void;
  toggleSound: () => void;
}

const byId = <T extends { id: string }>(list: T[]): Record<string, T> =>
  Object.fromEntries(list.map((x) => [x.id, x]));

function readTheme(): Theme {
  try {
    const saved = localStorage.getItem(THEME_KEY);
    if (saved === 'light' || saved === 'dark') return saved;
  } catch {
    // no storage (tests, private mode)
  }
  return 'dark';
}

function readSound(): boolean {
  try {
    return localStorage.getItem(SOUND_KEY) !== 'off';
  } catch {
    return true;
  }
}

function setThemeAttr(theme: Theme) {
  if (typeof document !== 'undefined') document.documentElement.dataset.theme = theme;
}

function applyTheme(theme: Theme) {
  setThemeAttr(theme);
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch {
    // ignore
  }
}

let toastSeq = 0;

// Put the saved theme on <html> before the first render.
const initialTheme = readTheme();
setThemeAttr(initialTheme);

export const useStore = create<State>()((set) => ({
  connected: false,
  daemonError: null,
  connEpoch: 0,
  projects: {},
  sessions: {},
  approvals: {},
  events: [],
  toasts: [],
  theme: initialTheme,
  soundEnabled: readSound(),

  applySnapshot: (s) =>
    set({
      projects: byId(s.projects),
      sessions: byId(s.sessions),
      approvals: byId(s.approvals.filter((a) => a.state === 'pending')),
    }),

  applyEvent: (e) =>
    set((st) => {
      switch (e.kind) {
        case 'session_output':
          return st;
        case 'session_updated':
          return { sessions: { ...st.sessions, [e.data.id]: e.data } };
        case 'project_added':
          return { projects: { ...st.projects, [e.data.id]: e.data } };
        case 'approval_created':
          return { approvals: { ...st.approvals, [e.data.id]: e.data } };
        case 'approval_resolved': {
          const { [e.data.id]: _gone, ...rest } = st.approvals;
          return { approvals: rest };
        }
        case 'event_appended':
          if (st.events.some((x) => x.seq === e.data.seq)) return st;
          return { events: [e.data, ...st.events].slice(0, MAX_EVENTS) };
        default:
          return st;
      }
    }),

  setConnected: (connected, error = null) =>
    set((st) => ({ connected, daemonError: connected ? null : error, connEpoch: connected && !st.connected ? st.connEpoch + 1 : st.connEpoch })),

  pushToast: (t) =>
    set((st) => {
      const toast: Toast = { id: ++toastSeq, msg: t.msg, kind: t.kind ?? 'idle', actions: t.actions };
      return { toasts: [...st.toasts, toast].slice(-4) };
    }),

  dismissToast: (id) => set((st) => ({ toasts: st.toasts.filter((t) => t.id !== id) })),

  toggleTheme: () =>
    set((st) => {
      const theme: Theme = st.theme === 'dark' ? 'light' : 'dark';
      applyTheme(theme);
      return { theme };
    }),

  toggleSound: () =>
    set((st) => {
      const soundEnabled = !st.soundEnabled;
      try {
        localStorage.setItem(SOUND_KEY, soundEnabled ? 'on' : 'off');
      } catch {
        // ignore
      }
      return { soundEnabled };
    }),
}));

type Slice = Pick<State, 'approvals' | 'sessions' | 'projects'>;

/** Pending approvals plus failed sessions. */
export const needsYouCount = (s: Pick<State, 'approvals' | 'sessions'>): number =>
  Object.values(s.approvals).filter((a) => a.state === 'pending').length +
  Object.values(s.sessions).filter((x) => x.status === 'failed').length;

/** Projects in the order they were added, each with its sessions, newest first. */
export const sessionsByProject = (s: Pick<Slice, 'projects' | 'sessions'>) =>
  Object.values(s.projects)
    .sort((a, b) => a.added_at - b.added_at || a.name.localeCompare(b.name))
    .map((project) => ({
      project,
      sessions: Object.values(s.sessions)
        .filter((x) => x.project_id === project.id)
        .sort((a, b) => b.started_at - a.started_at),
    }));

const pendingList = (s: Pick<State, 'approvals'>): Approval[] =>
  Object.values(s.approvals)
    .filter((a) => a.state === 'pending')
    .sort((a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id));

/** Pending approvals, oldest first. Returns a new array on every call. */
export const pendingApprovals = pendingList;

/** The pending approval that has waited longest. */
export const oldestPending = (s: Pick<State, 'approvals'>): Approval | undefined => pendingList(s)[0];

/** The oldest pending approval of one session. */
export const pendingForSession = (s: Pick<State, 'approvals'>, sessionId: string): Approval | undefined =>
  pendingList(s).find((a) => a.session_id === sessionId);

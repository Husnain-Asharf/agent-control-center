import type { EventRecord, Session } from './types';

const EDIT_TOOLS = ['Edit', 'Write', 'MultiEdit', 'NotebookEdit'];
/** Claude Code's tools for messages between local sessions. */
const MESSAGE_TOOLS = ['mcp__acc__send_message', 'SendMessage'];

export interface SessionWork {
  sessionId: string;
  /** Files it changed in the window, newest first, paths shown from the folder. */
  files: string[];
  messages: number;
}

export interface Coordination {
  /** Sessions sharing the project folder. */
  shared: SessionWork[];
  /** Sessions in their own worktree. */
  separate: SessionWork[];
  /** Files changed by two or more shared sessions, with who changed them. */
  overlaps: { file: string; sessionIds: string[] }[];
}

/** A path from the folder when it is inside it: `/repo/src/a.ts` in `/repo` is `src/a.ts`. */
export function fromFolder(path: string, folder: string): string {
  const norm = (p: string) => p.replace(/\\/g, '/').replace(/\/+$/, '');
  const p = norm(path);
  const f = norm(folder);
  return p.startsWith(`${f}/`) ? p.slice(f.length + 1) : p;
}

/** Who changed what in a project since `since`, from the event log. */
export function coordination(sessions: Session[], events: EventRecord[], since: number): Coordination {
  const work = new Map<string, SessionWork>();
  for (const s of sessions) work.set(s.id, { sessionId: s.id, files: [], messages: 0 });
  const byId = new Map(sessions.map((s) => [s.id, s]));
  // Newest first, so each file list starts with the latest change.
  const recent = events.filter((e) => e.kind === 'tool.used' && e.ts >= since && e.session_id).sort((a, b) => b.ts - a.ts);
  for (const e of recent) {
    const w = work.get(e.session_id ?? '');
    const s = byId.get(e.session_id ?? '');
    if (!w || !s) continue;
    const p = (e.payload ?? {}) as Record<string, unknown>;
    const tool = typeof p.tool === 'string' ? p.tool : '';
    const summary = typeof p.summary === 'string' ? p.summary : '';
    if (EDIT_TOOLS.includes(tool) && summary) {
      const file = fromFolder(summary, s.worktree);
      if (!w.files.includes(file)) w.files.push(file);
    } else if (MESSAGE_TOOLS.includes(tool)) {
      w.messages += 1;
    }
  }
  const shared = sessions.filter((s) => s.mode === 'project').map((s) => work.get(s.id)!);
  const separate = sessions.filter((s) => s.mode !== 'project').map((s) => work.get(s.id)!);
  const who = new Map<string, string[]>();
  for (const w of shared) for (const f of w.files) who.set(f, [...(who.get(f) ?? []), w.sessionId]);
  const overlaps = [...who.entries()]
    .filter(([, ids]) => ids.length > 1)
    .map(([file, sessionIds]) => ({ file, sessionIds }))
    .sort((a, b) => a.file.localeCompare(b.file));
  return { shared, separate, overlaps };
}

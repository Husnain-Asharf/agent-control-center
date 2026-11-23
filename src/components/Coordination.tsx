import { useEffect, useMemo, useState } from 'react';
import { Lamp } from './Lamp';
import { coordination, type SessionWork } from '../lib/coordination';
import { request } from '../lib/ipc';
import type { EventRecord, Project, Session } from '../lib/types';
import { useStore } from '../store/store';

/** How far back the view looks. */
const WINDOW_MS = 2 * 3_600_000;
const SHOWN_FILES = 5;

/** The project's sessions around its folder: who works where, on which files, and where they meet. */
export function Coordination({ project, sessions, now }: { project: Project; sessions: Session[]; now: number }) {
  const live = useStore((s) => s.events);
  const [loaded, setLoaded] = useState<EventRecord[]>([]);
  useEffect(() => {
    let on = true;
    request<EventRecord[]>('events_list', { session_id: null, limit: 500 })
      .then((list) => on && setLoaded(list))
      .catch(() => {});
    return () => {
      on = false;
    };
  }, [project.id]);
  const events = useMemo(() => {
    const seen = new Set<number>();
    return [...live, ...loaded].filter((e) => e.project_id === project.id && !seen.has(e.seq) && seen.add(e.seq));
  }, [live, loaded, project.id]);
  const c = useMemo(() => coordination(sessions, events, now - WINDOW_MS), [sessions, events, now]);
  const byId = useMemo(() => Object.fromEntries(sessions.map((s) => [s.id, s])), [sessions]);
  const branch = sessions.find((s) => s.mode === 'project')?.branch ?? project.base_branch;
  const overlapping = new Set(c.overlaps.flatMap((o) => o.file));

  if (sessions.length === 0) return null;
  return (
    <div className="coord">
      <div className="coord-folder">
        <strong>Project folder</strong>
        <span className="muted">
          on <code>{branch}</code> · {c.shared.length} {c.shared.length === 1 ? 'session' : 'sessions'} sharing it
        </span>
      </div>
      {c.shared.length > 0 && (
        <div className="coord-row">
          {c.shared.map((w) => (
            <Card key={w.sessionId} work={w} session={byId[w.sessionId]} overlapping={overlapping} />
          ))}
        </div>
      )}
      {c.overlaps.length > 0 && (
        <div className="coord-overlaps" role="status">
          {c.overlaps.map((o) => (
            <p key={o.file}>
              <code>{o.file}</code> was changed by {o.sessionIds.map((id) => byId[id]?.title ?? 'a session').join(' and ')}
            </p>
          ))}
        </div>
      )}
      {c.separate.length > 0 && (
        <>
          <p className="coord-label muted">In their own worktree</p>
          <div className="coord-row coord-row--separate">
            {c.separate.map((w) => (
              <Card key={w.sessionId} work={w} session={byId[w.sessionId]} overlapping={new Set()} />
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function Card({ work, session, overlapping }: { work: SessionWork; session: Session | undefined; overlapping: Set<string> }) {
  if (!session) return null;
  const more = work.files.length - SHOWN_FILES;
  return (
    <a className="coord-card" href={`#session/${session.id}`}>
      <span className="coord-title">
        <Lamp status={session.status} />
        <strong>{session.title}</strong>
      </span>
      {session.mode !== 'project' && (
        <span className="muted">
          on <code>{session.branch}</code>
        </span>
      )}
      {work.files.length === 0 ? (
        <span className="muted">No files changed in the last 2 hours</span>
      ) : (
        <ul>
          {work.files.slice(0, SHOWN_FILES).map((f) => (
            <li key={f} className={overlapping.has(f) ? 'is-overlap' : undefined}>
              <code>{f}</code>
            </li>
          ))}
          {more > 0 && <li className="muted">and {more} more</li>}
        </ul>
      )}
      {work.messages > 0 && (
        <span className="muted">
          Sent {work.messages} {work.messages === 1 ? 'message' : 'messages'} to other sessions
        </span>
      )}
    </a>
  );
}

import { useEffect, useMemo, useState } from 'react';
import { ApprovalDock } from '../components/ApprovalDock';
import { Lamp, STATUS_LABEL } from '../components/Lamp';
import { errorText } from '../lib/errorText';
import { eventText } from '../lib/eventText';
import { clock, fmtTimer, folderLabel } from '../lib/format';
import { fmtCost, level } from '../lib/usage';
import { request } from '../lib/ipc';
import { useRoute } from '../lib/route';
import type { EventRecord, Session } from '../lib/types';
import { pendingForSession, useStore } from '../store/store';
import { TerminalView } from '../terminal/TerminalView';
import { terminalKey } from '../terminal/terminalKey';

const LIVE: Session['status'][] = ['starting', 'working', 'needs_you', 'idle'];

function useEvents(sessionId: string): EventRecord[] {
  const [fetched, setFetched] = useState<EventRecord[]>([]);
  const live = useStore((s) => s.events);
  useEffect(() => {
    let off = false;
    setFetched([]);
    request<EventRecord[]>('events_list', { session_id: sessionId, limit: 100 })
      .then((r) => !off && setFetched(r))
      .catch(() => {});
    return () => {
      off = true;
    };
  }, [sessionId]);
  return useMemo(() => {
    const bySeq = new Map<number, EventRecord>();
    for (const ev of fetched) bySeq.set(ev.seq, ev);
    for (const ev of live) if (ev.session_id === sessionId) bySeq.set(ev.seq, ev);
    return [...bySeq.values()].sort((a, b) => b.seq - a.seq);
  }, [fetched, live, sessionId]);
}

export function SessionView() {
  const { param } = useRoute();
  const session = useStore((s) => (param ? s.sessions[param] : undefined));
  const project = useStore((s) => (session ? s.projects[session.project_id] : undefined));
  const pending = useStore((s) => (param ? pendingForSession(s, param) : undefined));
  const pushToast = useStore((s) => s.pushToast);
  const connEpoch = useStore((s) => s.connEpoch);
  const [now, setNow] = useState(() => Date.now());
  const events = useEvents(param ?? '');

  const running = session ? LIVE.includes(session.status) : false;
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [running]);

  if (!session) {
    return (
      <div className="screen">
        <nav className="crumbs" aria-label="Breadcrumb">
          <a href="#sessions">Sessions</a>
        </nav>
        <div className="empty">
          <h1>Session not found</h1>
          <p>It may have been removed. Go back to Sessions.</p>
        </div>
      </div>
    );
  }

  const act = (method: 'session_stop' | 'session_restart') =>
    request(method, { session_id: session.id }).catch((e: unknown) => pushToast({ msg: errorText(e), kind: 'failed' }));

  const elapsed = (running ? now : (session.ended_at ?? session.status_since)) - session.started_at;
  const folder = folderLabel(session);

  return (
    <div className="screen--flush agent">
      <header className="agent-head">
        <div className="agent-title">
          <Lamp status={session.status} />
          <h1>{session.title}</h1>
          <span className={`chip chip--${session.status === 'needs_you' ? 'needs' : session.status}`}>
            {STATUS_LABEL[session.status]}
          </span>
        </div>
        <div className="agent-actions">
          <button className="btn" type="button" disabled={running} onClick={() => act('session_restart')}>
            Restart
          </button>
          <button className="btn btn--danger" type="button" disabled={!running} onClick={() => act('session_stop')}>
            Stop
          </button>
        </div>
        <dl className="meta">
          <div>
            <dt>Agent</dt>
            <dd>{session.agent === 'claude' ? 'Claude Code' : session.agent}</dd>
          </div>
          <div>
            <dt>Project</dt>
            <dd>
              {project ? (
                <a className="link" href={`#project/${project.id}`}>
                  {project.name}
                </a>
              ) : (
                'Unknown project'
              )}
            </dd>
          </div>
          <div>
            <dt>Branch</dt>
            <dd>
              <code>{session.branch}</code>
            </dd>
          </div>
          <div>
            <dt>Folder</dt>
            <dd>
              <code>{folder}</code>
            </dd>
          </div>
          {session.usage && (
            <>
              <div>
                <dt>Cost</dt>
                <dd title="What the same use would cost on the API">{fmtCost(session.usage.cost_usd)}</dd>
              </div>
              {session.usage.context_used_percentage != null && (
                <div>
                  <dt>Context</dt>
                  <dd className={`ctx ctx--${level(session.usage.context_used_percentage)}`}>
                    {Math.round(session.usage.context_used_percentage)}% full
                  </dd>
                </div>
              )}
              {session.usage.model && (
                <div>
                  <dt>Model</dt>
                  <dd>
                    {session.usage.model}
                    {session.usage.effort ? ` · ${session.usage.effort}` : ''}
                  </dd>
                </div>
              )}
              <div>
                <dt>Lines</dt>
                <dd>
                  +{session.usage.lines_added} −{session.usage.lines_removed}
                </dd>
              </div>
            </>
          )}
          <div>
            <dt>Running</dt>
            <dd>{fmtTimer(elapsed)}</dd>
          </div>
        </dl>
      </header>
      {pending && (
        <ApprovalDock
          approval={pending}
          projectName={project?.name ?? 'this project'}
          baseBranch={project?.base_branch ?? 'the base branch'}
        />
      )}
      <div className="agent-body">
        <div className="term-col">
          <TerminalView key={terminalKey(session, connEpoch)} sessionId={session.id} />
        </div>
        <aside className="inspector" aria-label="Session details">
          <div className="tabs" role="tablist">
            <button className="tab" type="button" role="tab" aria-selected="true">
              Activity
            </button>
          </div>
          <div className="inspector-body" role="tabpanel">
            {events.length === 0 ? (
              <p className="note">No activity yet.</p>
            ) : (
              <ol className="events">
                {events.map((ev) => (
                  <li className="event" key={ev.seq}>
                    <time>{clock(ev.ts)}</time>
                    <span className={`dot${ev.actor === 'user' ? ' dot--user' : ''}`} />
                    <span>{eventText(ev)}</span>
                  </li>
                ))}
              </ol>
            )}
          </div>
        </aside>
      </div>
    </div>
  );
}

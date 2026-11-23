import { useEffect, useMemo, useState } from 'react';
import { ApprovalDock } from '../components/ApprovalDock';
import { Lamp, STATUS_LABEL } from '../components/Lamp';
import { errorText } from '../lib/errorText';
import { eventText } from '../lib/eventText';
import { clock, fmtTimer, folderLabel } from '../lib/format';
import { fmtCost, level } from '../lib/usage';
import { fmtBytes, useStats } from '../lib/stats';
import { Dialog } from '../components/Dialog';
import { go } from '../lib/route';
import type { SessionRemoved } from '../lib/types';
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
  const [removing, setRemoving] = useState(false);
  const stats = useStats();
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
          <p>This session is not in the app. Go back to Sessions.</p>
        </div>
      </div>
    );
  }

  const act = (method: 'session_stop' | 'session_restart') =>
    request(method, { session_id: session.id }).catch((e: unknown) => pushToast({ msg: errorText(e), kind: 'failed' }));

  const mine = stats?.sessions.find((r) => r.session_id === session.id);
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
          {!running && (
            <button className="btn btn--ghost" type="button" onClick={() => setRemoving(true)}>
              Remove
            </button>
          )}
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
          {session.mode === 'project' && session.stay_on_base && (
            <div>
              <dt>Base branch</dt>
              <dd title="You chose Allow on the base branch, so this session is not asked again">Changes allowed</dd>
            </div>
          )}
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
          {mine && fmtBytes(mine.memory_bytes) && (
            <div>
              <dt>Memory</dt>
              <dd title={`${mine.processes} ${mine.processes === 1 ? 'process' : 'processes'}: Claude Code and what it started`}>
                {fmtBytes(mine.memory_bytes)}
              </dd>
            </div>
          )}
          {mine && (fmtBytes(mine.transcript_bytes) || fmtBytes(mine.folder_bytes)) && (
            <div>
              <dt>Disk</dt>
              <dd title="The conversation file, and the session's own worktree if it has one">
                {[fmtBytes(mine.transcript_bytes) && `${fmtBytes(mine.transcript_bytes)} chat`, fmtBytes(mine.folder_bytes) && `${fmtBytes(mine.folder_bytes)} worktree`]
                  .filter(Boolean)
                  .join(' · ')}
              </dd>
            </div>
          )}
          <div>
            <dt>{running ? 'Running' : 'Ran for'}</dt>
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
      {removing && <RemoveDialog session={session} onClose={() => setRemoving(false)} />}
    </div>
  );
}

function RemoveDialog({ session, onClose }: { session: Session; onClose: () => void }) {
  const pushToast = useStore((s) => s.pushToast);
  const own = session.mode !== 'project';
  const [alsoWorktree, setAlsoWorktree] = useState(false);
  const [busy, setBusy] = useState(false);
  const submit = async () => {
    setBusy(true);
    try {
      const r = await request<SessionRemoved>('session_remove', { session_id: session.id, remove_worktree: own && alsoWorktree });
      onClose();
      go('#sessions');
      if (own && alsoWorktree) {
        const msg = r.worktree_removed
          ? r.branch_removed
            ? `Removed ${session.title} with its worktree and branch.`
            : `Removed ${session.title} and its worktree. The branch ${session.branch} was kept: ${r.kept ?? 'git kept it'}.`
          : `Removed ${session.title}. Its worktree was kept: ${r.kept ?? 'git kept it'}.`;
        pushToast({ msg, kind: r.worktree_removed && r.branch_removed ? 'done' : 'needs' });
      }
    } catch (e) {
      pushToast({ msg: errorText(e), kind: 'failed' });
      setBusy(false);
    }
  };
  return (
    <Dialog title="Remove session" onClose={onClose} onSubmit={submit}>
      <p className="muted">
        {session.title} leaves the app with its history. Its files and commits stay where they are.
      </p>
      {own && (
        <label className="check">
          <input type="checkbox" checked={alsoWorktree} onChange={(e) => setAlsoWorktree(e.target.checked)} />
          Also delete its worktree and the branch {session.branch}, if git can do it without losing work
        </label>
      )}
      <div className="dialog-actions">
        <button type="button" className="btn btn--ghost" onClick={onClose}>
          Cancel
        </button>
        <button type="submit" className="btn btn--danger" disabled={busy}>
          Remove
        </button>
      </div>
    </Dialog>
  );
}

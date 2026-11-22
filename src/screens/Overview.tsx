import { useMemo, useState } from 'react';
import { FleetLanes, RangeControl, byRank, useNow, type Range } from '../components/FleetLanes';
import { Icon } from '../components/Icon';
import { Lamp } from '../components/Lamp';
import { openNewSession } from '../dialogs/NewSessionDialog';
import { fmtDur } from '../lib/format';
import { decide, isProtectedApproval, wantsVerb } from '../lib/approvals';
import { segmentsFor } from '../lib/segments';
import { useStore } from '../store/store';

const HOUR = 3_600_000;

type Item =
  | { kind: 'approval'; id: string; at: number; sessionId: string; tool: string; summary: string; protected: boolean }
  | { kind: 'failed'; id: string; at: number; sessionId: string };

export function Overview() {
  const projects = useStore((s) => s.projects);
  const sessions = useStore((s) => s.sessions);
  const approvals = useStore((s) => s.approvals);
  const [range, setRange] = useState<Range>(60);
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const now = useNow();

  const list = useMemo(() => Object.values(sessions).sort(byRank), [sessions]);
  const items = useMemo(() => {
    const out: Item[] = [
      ...Object.values(approvals)
        .filter((a) => a.state === 'pending')
        .map((a): Item => ({ kind: 'approval', id: a.id, at: a.created_at, sessionId: a.session_id, tool: a.tool, summary: a.summary, protected: isProtectedApproval(a) })),
      ...Object.values(sessions)
        .filter((s) => s.status === 'failed')
        .map((s): Item => ({ kind: 'failed', id: s.id, at: s.status_since, sessionId: s.id })),
    ];
    return out.sort((a, b) => a.at - b.at);
  }, [approvals, sessions]);

  const projectCount = new Set(list.map((s) => s.project_id)).size;
  const waited = list.reduce(
    (sum, s) =>
      sum +
      segmentsFor(s.history, now, HOUR)
        .filter((g) => g.kind === 'needs')
        .reduce((n, g) => n + ((g.end ?? now) - g.start), 0),
    0,
  );

  const allowOnce = async (approvalId: string) => {
    setBusy((b) => ({ ...b, [approvalId]: true }));
    try {
      await decide(approvalId, 'allow_once');
    } finally {
      setBusy((b) => ({ ...b, [approvalId]: false }));
    }
  };

  const who = (sessionId: string) => {
    const s = sessions[sessionId];
    return { title: s?.title ?? 'Unknown session', project: s ? (projects[s.project_id]?.name ?? 'unknown project') : '' };
  };

  return (
    <div className="screen">
      <div className="screen-head">
        <div>
          <h1>Overview</h1>
          <p>
            {list.length} {list.length === 1 ? 'session' : 'sessions'} in {projectCount}{' '}
            {projectCount === 1 ? 'project' : 'projects'}
          </p>
        </div>
        <button className="btn btn--primary" type="button" onClick={openNewSession}>
          <Icon name="plus" />
          New session
          <kbd className="kbd">N</kbd>
        </button>
      </div>
      <section className="block" aria-labelledby="needs-h">
        <div className="block-head">
          <h2 id="needs-h">Needs you</h2>
          <p>Oldest first</p>
        </div>
        <div className="list">
          {items.length === 0 ? (
            <p className="empty-line">Nothing needs you. Sessions ask here when they need an approval or an answer.</p>
          ) : (
            items.map((it) => {
              const w = who(it.sessionId);
              if (it.kind === 'failed') {
                return (
                  <div className="needs-row needs-row--failed" key={`f-${it.id}`}>
                    <Lamp status="failed" />
                    <a className="who" href={`#session/${it.id}`}>
                      <strong>{w.title}</strong>
                      <span>Claude Code in {w.project}</span>
                    </a>
                    <div className="what">Failed</div>
                    <div className="wait wait--failed">{fmtDur(now - it.at)} ago</div>
                    <div className="row-actions">
                      <a className="btn" href={`#session/${it.id}`}>
                        Open
                      </a>
                    </div>
                  </div>
                );
              }
              const protectedItem = it.protected;
              return (
                <div className="needs-row" key={`a-${it.id}`}>
                  <Lamp status="needs_you" />
                  <a className="who" href={`#session/${it.sessionId}`}>
                    <strong>{w.title}</strong>
                    <span>Claude Code in {w.project}</span>
                  </a>
                  <div className="what">
                    Asked to {wantsVerb(it.tool)} <code>{it.summary}</code>
                  </div>
                  <div className="wait">{fmtDur(now - it.at)}</div>
                  <div className="row-actions">
                    {protectedItem ? (
                      <a className="btn" href={`#approvals/${it.id}`}>
                        Open
                      </a>
                    ) : (
                      <>
                        <button className="btn" type="button" disabled={busy[it.id]} onClick={() => allowOnce(it.id)}>
                          Allow once
                        </button>
                        <a className="btn" href={`#approvals/${it.id}`}>
                          Open
                        </a>
                      </>
                    )}
                  </div>
                </div>
              );
            })
          )}
        </div>
      </section>
      <section className="block" aria-labelledby="fleet-h">
        <div className="block-head">
          <h2 id="fleet-h">Fleet</h2>
          <div className="block-tools">
            <p>
              Sessions waited <strong>{fmtDur(waited)}</strong> for you in the last hour
            </p>
            <RangeControl range={range} onChange={setRange} />
          </div>
        </div>
        {list.length === 0 ? (
          <div className="list">
            <p className="empty-line">No sessions yet. Start a session and its lane shows up here.</p>
          </div>
        ) : (
          <FleetLanes sessions={list} projects={projects} range={range} now={now} />
        )}
      </section>
    </div>
  );
}

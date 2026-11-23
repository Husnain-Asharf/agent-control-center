import { useEffect, useMemo, useRef, useState } from 'react';
import { Lamp } from '../components/Lamp';
import {
  alwaysAllowState,
  approvalKey,
  branchQuestionBase,
  decide,
  decisionFor,
  decisionKeyAllowed,
  decisionKeysReady,
  isBranchQuestion,
  useNow,
  wantsText,
} from '../lib/approvals';
import { fmtTimer } from '../lib/format';
import { go, useRoute } from '../lib/route';
import type { Approval, Decision } from '../lib/types';
import { pendingApprovals, useStore } from '../store/store';

function ListItem({ ap, selected, now }: { ap: Approval; selected: boolean; now: number }) {
  const session = useStore((s) => s.sessions[ap.session_id]);
  const project = useStore((s) => (session ? s.projects[session.project_id] : undefined));
  return (
    <button
      className="ap-item"
      type="button"
      role="option"
      aria-selected={selected}
      onClick={() => go(`#approvals/${ap.id}`)}
    >
      <span className="lamp lamp--needs" />
      <span className="ap-main">
        <span className="ap-title">{ap.summary}</span>
        <span className="ap-sub">{session?.title ?? 'Unknown session'}</span>
        <span className="ap-sub faint">
          Claude Code in {project?.name ?? 'unknown project'}
          {isBranchQuestion(ap) && <> · on {project?.base_branch ?? 'the base branch'}</>}
        </span>
      </span>
      <span className="ap-wait">{fmtTimer(now - ap.created_at)}</span>
    </button>
  );
}

function Detail({ ap, now }: { ap: Approval; now: number }) {
  const session = useStore((s) => s.sessions[ap.session_id]);
  const project = useStore((s) => (session ? s.projects[session.project_id] : undefined));
  const [busy, setBusy] = useState(false);
  const projectName = project?.name ?? 'this project';
  const base = branchQuestionBase(ap, project?.base_branch ?? 'the base branch');
  const branchQuestion = isBranchQuestion(ap);
  const always = alwaysAllowState(ap);
  const json = useMemo(() => JSON.stringify(ap.input, null, 2) ?? 'null', [ap.input]);

  const act = (d: Decision) => {
    setBusy(true);
    decide(ap.id, d).finally(() => setBusy(false));
  };

  return (
    <>
      <div className="detail">
        {session ? (
          <a className="detail-agent" href={`#session/${session.id}`}>
            <Lamp status={session.status} />
            <strong>{session.title}</strong>
            <span className="muted">Claude Code in {projectName}</span>
          </a>
        ) : (
          <span className="detail-agent">
            <span className="lamp lamp--needs" />
            <strong>Unknown session</strong>
          </span>
        )}
        <h2 className="detail-title">
          {wantsText(ap.tool)}
          {branchQuestion && <> on {base}</>}
        </h2>
        {branchQuestion && (
          <p className="muted">
            The project folder is on {base}. Make a branch first to keep {base} as it is, or allow this change on{' '}
            {base}.
          </p>
        )}
        <pre className="well request">
          <code>{ap.summary}</code>
        </pre>
        <h3 className="sub-title">Raw input</h3>
        <pre className="well request">
          <code>{json}</code>
        </pre>
        <dl className="kv">
          <dt>Folder</dt>
          <dd>
            <code>{session?.worktree ?? 'Unknown'}</code>
          </dd>
          <dt>Waiting</dt>
          <dd>{fmtTimer(now - ap.created_at)}</dd>
        </dl>
      </div>
      {branchQuestion ? (
        <div className="decide-bar">
          <button className="btn btn--primary btn--lg" type="button" disabled={busy} onClick={() => act('make_branch')}>
            Make branch {ap.branch ?? ''} <kbd className="kbd">A</kbd>
          </button>
          <button className="btn btn--lg" type="button" disabled={busy} onClick={() => act('stay_on_base')}>
            Allow on {base} <kbd className="kbd">⇧A</kbd>
          </button>
          <button className="btn btn--lg btn--danger" type="button" disabled={busy} onClick={() => act('deny')}>
            Deny <kbd className="kbd">D</kbd>
          </button>
          <p className="decide-note">
            Make branch starts {ap.branch ?? 'a branch'} at the current commit and keeps uncommitted changes. Every
            session in this folder goes on with it. Allow on {base} lets this session change {base}, and it is not
            asked again.
          </p>
        </div>
      ) : (
        <div className="decide-bar">
          <button className="btn btn--primary btn--lg" type="button" disabled={busy} onClick={() => act('allow_once')}>
            Allow once <kbd className="kbd">A</kbd>
          </button>
          <button
            className="btn btn--lg"
            type="button"
            disabled={busy || !always.enabled}
            title={always.note ?? undefined}
            onClick={() => act('allow_always')}
          >
            Always allow in {projectName} <kbd className="kbd">⇧A</kbd>
          </button>
          <button className="btn btn--lg btn--danger" type="button" disabled={busy} onClick={() => act('deny')}>
            Deny <kbd className="kbd">D</kbd>
          </button>
          <p className="decide-note">
            {always.enabled ? `Always allow adds a rule for this project: ${always.preview}.` : `${always.note}.`}
          </p>
        </div>
      )}
    </>
  );
}

export function Approvals() {
  const { param } = useRoute();
  const approvals = useStore((s) => s.approvals);
  const list = useMemo(() => pendingApprovals({ approvals }), [approvals]);
  const now = useNow();
  const sel = list.find((a) => a.id === param) ?? list[0];

  // Keep the address in step with what is shown, so a reload or a copied link lands on the same item.
  useEffect(() => {
    const want = sel ? `#approvals/${sel.id}` : '#approvals';
    if (window.location.hash !== want) window.history.replaceState(null, '', want);
  }, [sel]);

  // True while a decision sent from the keyboard has not come back yet.
  const keyBusy = useRef(false);
  // When the shown item last changed. Decision keys wait 400 ms after that.
  const selected = useRef({ id: sel?.id, since: Date.now() });
  if (selected.current.id !== sel?.id) selected.current = { id: sel?.id, since: Date.now() };
  const latest = useRef({ list, sel });
  latest.current = { list, sel };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const k = approvalKey({
        key: e.key,
        metaKey: e.metaKey,
        ctrlKey: e.ctrlKey,
        altKey: e.altKey,
        shiftKey: e.shiftKey,
        repeat: e.repeat,
        defaultPrevented: e.defaultPrevented,
        target: e.target as HTMLElement | null,
      });
      const { list, sel } = latest.current;
      if (!k || !sel) return;
      if (k === 'next' || k === 'prev') {
        const i = list.findIndex((a) => a.id === sel.id);
        const nx = list[Math.min(list.length - 1, Math.max(0, i + (k === 'next' ? 1 : -1)))];
        e.preventDefault();
        if (nx) go(`#approvals/${nx.id}`);
      } else {
        if (!decisionKeyAllowed(k, !!document.querySelector('[role="dialog"]'))) return;
        e.preventDefault();
        if (keyBusy.current || !decisionKeysReady(Date.now(), selected.current.since)) return;
        keyBusy.current = true;
        void decide(sel.id, decisionFor(sel, k)).finally(() => {
          keyBusy.current = false;
        });
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  return (
    <div className="screen--flush two-pane">
      <div className="pane-list">
        <div className="pane-head">
          <h1>Approvals</h1>
          <p className="muted">{list.length ? `${list.length} waiting, oldest first` : 'Nothing waiting'}</p>
        </div>
        <div role="listbox" aria-label="Waiting requests">
          {list.map((ap) => (
            <ListItem key={ap.id} ap={ap} selected={ap.id === sel?.id} now={now} />
          ))}
        </div>
      </div>
      <div className="pane-detail">
        {sel ? (
          <Detail key={sel.id} ap={sel} now={now} />
        ) : (
          <div className="empty">
            <h2>Nothing is waiting</h2>
            <p>Sessions ask here when they need an approval.</p>
          </div>
        )}
      </div>
    </div>
  );
}

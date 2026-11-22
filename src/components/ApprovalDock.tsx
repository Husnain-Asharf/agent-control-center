import { useState } from 'react';
import { alwaysAllowState, decide, useNow, wantsVerb } from '../lib/approvals';
import { fmtTimer } from '../lib/format';
import type { Approval, Decision } from '../lib/types';

/** The pending request of one session, with the same actions as the inbox. */
export function ApprovalDock({ approval, projectName }: { approval: Approval; projectName: string }) {
  const now = useNow();
  const [busy, setBusy] = useState(false);
  const always = alwaysAllowState(approval);

  const act = (d: Decision) => {
    setBusy(true);
    decide(approval.id, d).finally(() => setBusy(false));
  };

  return (
    <div className="dock" role="region" aria-label="Approval needed">
      <span className="lamp lamp--needs" />
      <div className="dock-main">
        <div className="dock-title">
          <strong>Wants to {wantsVerb(approval.tool)}</strong> <code>{approval.summary}</code>
        </div>
        <div className="dock-sub">
          Waiting {fmtTimer(now - approval.created_at)}
          {always.enabled ? <> · Always allow covers {always.preview}</> : <> · {always.note}</>}
        </div>
      </div>
      <div className="dock-actions">
        <button className="btn btn--primary" type="button" disabled={busy} onClick={() => act('allow_once')}>
          Allow once
        </button>
        <button
          className="btn"
          type="button"
          disabled={busy || !always.enabled}
          title={always.note ?? undefined}
          onClick={() => act('allow_always')}
        >
          Always allow in {projectName}
        </button>
        <button className="btn btn--danger" type="button" disabled={busy} onClick={() => act('deny')}>
          Deny
        </button>
        <a className="btn btn--ghost" href={`#approvals/${approval.id}`}>
          Details
        </a>
      </div>
    </div>
  );
}

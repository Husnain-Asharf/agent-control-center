import { useEffect, useState } from 'react';
import { clock, fmtDur } from '../lib/format';
import { marksFor, segmentsFor } from '../lib/segments';
import type { Project, Session, SessionStatus } from '../lib/types';
import { Lamp, STATUS_LABEL } from './Lamp';

const MIN = 60_000;

export const RANGES = [15, 60, 240] as const;
export type Range = (typeof RANGES)[number];

/** Current time in ms, refreshed on an interval so lanes keep moving. */
export function useNow(everyMs = 5000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(t);
  }, [everyMs]);
  return now;
}

export function RangeControl({ range, onChange }: { range: Range; onChange: (r: Range) => void }) {
  return (
    <div className="seg-mini" role="group" aria-label="Time range">
      {RANGES.map((r) => (
        <button key={r} type="button" className="seg-mini-btn" aria-pressed={range === r} onClick={() => onChange(r)}>
          {r < 60 ? `${r}m` : `${r / 60}h`}
        </button>
      ))}
    </div>
  );
}

function ticks(now: number, range: Range): { pct: number; label: string }[] {
  const windowMs = range * MIN;
  const start = now - windowMs;
  const step = (range <= 15 ? 5 : range <= 60 ? 10 : 30) * MIN;
  const out: { pct: number; label: string }[] = [];
  for (let t = Math.ceil(start / step) * step; t < now - windowMs * 0.05; t += step) {
    out.push({ pct: ((t - start) / windowMs) * 100, label: clock(t) });
  }
  return out;
}

const RANK: Record<SessionStatus, number> = {
  needs_you: 0,
  failed: 1,
  working: 2,
  starting: 2,
  idle: 3,
  done: 4,
  stopped: 5,
};

/** Sessions that need you first, then working, idle, done and stopped; newest first within a group. */
export const byRank = (a: Session, b: Session): number =>
  RANK[a.status] - RANK[b.status] || b.started_at - a.started_at;

function statusText(s: Session, now: number): string {
  const label = STATUS_LABEL[s.status];
  return s.status === 'needs_you' || s.status === 'working' ? `${label} ${fmtDur(now - s.status_since)}` : label;
}

const SEG_LABEL = { work: 'Working', needs: 'Waiting for you', idle: 'Idle' } as const;

function Track({ session, now, range }: { session: Session; now: number; range: Range }) {
  const windowMs = range * MIN;
  const start = now - windowMs;
  const pct = (t: number) => Math.min(100, Math.max(0, ((t - start) / windowMs) * 100));
  const segs = segmentsFor(session.history, now, windowMs);
  const mark = marksFor(session);
  const endT = session.ended_at ?? now;
  return (
    <span className="track">
      {ticks(now, range).map((t) => (
        <span className="tick" key={t.pct} style={{ left: `${t.pct}%` }} />
      ))}
      {endT > start && (
        <span
          className="track-base"
          style={{ left: `${pct(session.started_at)}%`, width: `${pct(endT) - pct(session.started_at)}%` }}
        />
      )}
      {segs.map((s) => {
        const open = s.end === null;
        const left = pct(s.start);
        const width = Math.max(0.5, pct(s.end ?? now) - left);
        const label = s.kind === 'needs' && !open ? 'Waited for you' : SEG_LABEL[s.kind];
        const span = open ? `since ${clock(s.start)}` : `${clock(s.start)} to ${clock(s.end as number)}`;
        return (
          <span
            key={`${s.kind}-${s.start}`}
            className={`seg seg--${s.kind}${open ? ' is-open' : ''}`}
            style={{ left: `${left}%`, width: `${width}%` }}
            title={`${label}, ${span}`}
          />
        );
      })}
      {mark && mark.at > start && (
        <span
          className={`mark mark--${mark.kind}`}
          style={{ left: `${pct(mark.at)}%` }}
          title={`${mark.kind === 'done' ? 'Done' : 'Failed'} at ${clock(mark.at)}`}
        />
      )}
    </span>
  );
}

const STATUS_CLASS: Partial<Record<SessionStatus, string>> = {
  needs_you: ' lane-status--needs',
  failed: ' lane-status--failed',
  done: ' lane-status--done',
};

export function FleetLanes({
  sessions,
  projects,
  range,
  now,
}: {
  sessions: Session[];
  projects: Record<string, Project>;
  range: Range;
  now: number;
}) {
  return (
    <>
      <div className="well lanes">
        <div className="lane-row lane-axis" aria-hidden="true">
          <span>Session</span>
          <div className="axis">
            {ticks(now, range).map((t) => (
              <span key={t.pct} style={{ left: `${t.pct}%` }}>
                {t.label}
              </span>
            ))}
            <span className="axis-now" style={{ left: '100%' }}>
              Now
            </span>
          </div>
          <span>Status</span>
          <span className="lane-cost" />
        </div>
        <div className="lanes-body">
          {sessions.map((s) => (
            <a
              key={s.id}
              className="lane-row lane"
              href={`#session/${s.id}`}
              aria-label={`${s.title}: ${STATUS_LABEL[s.status]}`}
            >
              <span className="lane-label">
                <Lamp status={s.status} />
                <span className="who">
                  <strong>{s.title}</strong>
                  <span>Claude Code in {projects[s.project_id]?.name ?? 'unknown project'}</span>
                </span>
              </span>
              <Track session={s} now={now} range={range} />
              <span className={`lane-status${STATUS_CLASS[s.status] ?? ''}`}>{statusText(s, now)}</span>
              <span className="lane-cost" />
            </a>
          ))}
        </div>
      </div>
      <div className="legend">
        <span><i className="key key--work" />Working</span>
        <span><i className="key key--needs" />Waiting for you</span>
        <span><i className="key key--idle" />Idle</span>
        <span><i className="key key--done" />Done</span>
        <span><i className="key key--failed" />Failed</span>
      </div>
    </>
  );
}

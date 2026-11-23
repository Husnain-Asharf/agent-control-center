import { useEffect, useMemo, useState } from 'react';
import { useNow } from '../components/FleetLanes';
import { Dialog } from '../components/Dialog';
import { Lamp, STATUS_LABEL } from '../components/Lamp';
import { TrendChart } from '../components/TrendChart';
import {
  RANGE_IDS,
  RANGE_LABEL,
  RANGE_MS,
  approvalsAsked,
  buildRows,
  compactTargets,
  costInRange,
  costSeries,
  hasPoints,
  memoryNow,
  memorySeries,
  niceBytes,
  planSeries,
  runningCount,
  sortRows,
  sumValues,
  type RangeId,
  type SessionRow,
  type SortKey,
} from '../lib/analytics';
import { bytesToB64 } from '../lib/b64';
import { errorText } from '../lib/errorText';
import { fmtDur } from '../lib/format';
import { request } from '../lib/ipc';
import { canCompact, isRunning, PLAN_HIGH_AT, planHigh } from '../lib/recommend';
import { fmtBytes, useStats } from '../lib/stats';
import type { EventRecord, PlanLimit, UsageSample } from '../lib/types';
import { fmtCost, level, resetText } from '../lib/usage';
import { useStore } from '../store/store';

const RANGE_TEXT: Record<RangeId, string> = { '1h': 'last hour', '24h': 'last 24 hours', '7d': 'last 7 days' };

const pct = (v: number) => `${Math.round(v)}%`;
const money = (v: number) => (v === 0 ? '$0' : v < 10 ? `$${v.toFixed(2)}` : `$${Math.round(v)}`);
const bytes = (v: number) => (v === 0 ? '0' : (fmtBytes(v) ?? ''));

const encoder = new TextEncoder();
const sendCompact = (sessionId: string) =>
  request('session_input', { session_id: sessionId, data_b64: bytesToB64(encoder.encode('/compact\r')) });

/** Samples and events for the range, asked again every minute while the screen is open. */
function useHistory(since: number): { samples: UsageSample[] | null; events: EventRecord[] } {
  const [samples, setSamples] = useState<UsageSample[] | null>(null);
  const [events, setEvents] = useState<EventRecord[]>([]);
  useEffect(() => {
    let live = true;
    const ask = () => {
      request<UsageSample[]>('usage_history', { since })
        .then((s) => live && setSamples(s))
        .catch(() => live && setSamples((prev) => prev ?? []));
      request<EventRecord[]>('events_list', { session_id: null, limit: 500 })
        .then((e) => live && setEvents(e))
        .catch(() => {});
    };
    ask();
    const t = setInterval(ask, 60_000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [since]);
  return { samples, events };
}

export function Analytics() {
  const projects = useStore((s) => s.projects);
  const sessions = useStore((s) => s.sessions);
  const approvals = useStore((s) => s.approvals);
  const planUsage = useStore((s) => s.planUsage);
  const liveEvents = useStore((s) => s.events);
  const pushToast = useStore((s) => s.pushToast);
  const stats = useStats(5000);
  const now = useNow(5000);
  const [range, setRange] = useState<RangeId>('24h');
  const [costBy, setCostBy] = useState<'total' | 'project'>('total');
  const [sort, setSort] = useState<{ key: SortKey; dir: 'asc' | 'desc' }>({ key: 'cost', dir: 'desc' });
  const [confirm, setConfirm] = useState(false);
  const [sending, setSending] = useState<Record<string, boolean>>({});

  // The fetch window moves only when the range changes or once a minute, so polling stays steady.
  const minute = Math.floor(now / 60_000);
  const since = useMemo(() => minute * 60_000 - RANGE_MS[range], [minute, range]);
  const { samples, events } = useHistory(since);
  const list = useMemo(() => samples ?? [], [samples]);

  const allEvents = useMemo(() => {
    const seen = new Set(events.map((e) => e.seq));
    return [...events, ...liveEvents.filter((e) => !seen.has(e.seq))];
  }, [events, liveEvents]);

  const sessionList = useMemo(() => Object.values(sessions), [sessions]);
  const costs = useMemo(() => costInRange(list, sessions, since), [list, sessions, since]);
  const asked = useMemo(() => approvalsAsked(allEvents, Object.values(approvals), since), [allEvents, approvals, since]);
  const rows = useMemo(
    () => buildRows({ sessions: sessionList, projects, stats, asked, costs, now, rangeMs: RANGE_MS[range] }),
    [sessionList, projects, stats, asked, costs, now, range],
  );
  const sorted = useMemo(() => sortRows(rows, sort.key, sort.dir), [rows, sort]);
  const targets = useMemo(() => compactTargets(rows), [rows]);

  const plan = useMemo(() => planSeries(list, since), [list, since]);
  const cost = useMemo(() => costSeries(list, sessions, projects, since, costBy), [list, sessions, projects, since, costBy]);
  const memory = useMemo(() => [memorySeries(list, since)], [list, since]);
  const anySamples = hasPoints(plan) || hasPoints(cost) || hasPoints(memory);

  const memNow = memoryNow(stats);
  const fiveHour = planUsage?.five_hour ?? null;

  const compact = async (ids: string[]) => {
    setSending((s) => ({ ...s, ...Object.fromEntries(ids.map((id) => [id, true])) }));
    const results = await Promise.allSettled(ids.map((id) => sendCompact(id)));
    setSending((s) => ({ ...s, ...Object.fromEntries(ids.map((id) => [id, false])) }));
    const failed = results.find((r): r is PromiseRejectedResult => r.status === 'rejected');
    const ok = results.length - results.filter((r) => r.status === 'rejected').length;
    if (ok > 0) {
      const name = ids.length === 1 ? (sessions[ids[0]]?.title ?? 'the session') : `${ok} sessions`;
      pushToast({ msg: `Sent /compact to ${name}.`, kind: 'idle' });
    }
    if (failed) pushToast({ msg: errorText(failed.reason), kind: 'failed' });
  };

  const sortBy = (key: SortKey) =>
    setSort((s) => (s.key === key ? { key, dir: s.dir === 'desc' ? 'asc' : 'desc' } : { key, dir: 'desc' }));

  const th = (k: SortKey, label: string) => (
    <th key={k} className="num" aria-sort={sort.key === k ? (sort.dir === 'asc' ? 'ascending' : 'descending') : 'none'}>
      <button type="button" className="an-sort" onClick={() => sortBy(k)}>
        {label}
        <span className="an-sort-dir" aria-hidden="true">
          {sort.key === k ? (sort.dir === 'asc' ? '↑' : '↓') : ''}
        </span>
      </button>
    </th>
  );

  return (
    <div className="screen an">
      <div className="screen-head">
        <div>
          <h1>Analytics</h1>
          <p>Use, cost and memory across your sessions, {RANGE_TEXT[range]}.</p>
        </div>
        <div className="seg-mini" role="group" aria-label="Time range">
          {RANGE_IDS.map((r) => (
            <button key={r} type="button" className="seg-mini-btn" aria-pressed={range === r} onClick={() => setRange(r)}>
              {RANGE_LABEL[r]}
            </button>
          ))}
        </div>
      </div>

      {planHigh(planUsage) && fiveHour && (
        <p className="an-banner" role="status">
          The 5-hour window is at {pct(fiveHour.used_percentage)}
          {resetText(fiveHour.resets_at, now) ? `, and resets ${resetText(fiveHour.resets_at, now)}` : ''}. Sessions stop
          when it runs out, so hold off on new work or compact long sessions.
        </p>
      )}

      <div className="an-tiles">
        <LimitTile label="5-hour use" limit={fiveHour} now={now} />
        <LimitTile label="Weekly use" limit={planUsage?.seven_day ?? null} now={now} />
        <div className="an-tile">
          <span className="an-tile-label">Cost, {RANGE_TEXT[range]}</span>
          <strong className="an-tile-value">{fmtCost(sumValues(costs))}</strong>
          <span className="an-tile-sub">At API prices</span>
        </div>
        <div className="an-tile">
          <span className="an-tile-label">Sessions running</span>
          <strong className="an-tile-value">{runningCount(sessionList)}</strong>
          <span className="an-tile-sub">of {sessionList.length}</span>
        </div>
        <div className="an-tile">
          <span className="an-tile-label">Memory in use</span>
          <strong className="an-tile-value">{memNow == null ? 'None' : fmtBytes(memNow)}</strong>
          <span className="an-tile-sub">All running sessions</span>
        </div>
        <div className="an-tile">
          <span className="an-tile-label">App data</span>
          <strong className="an-tile-value">{stats ? fmtBytes(stats.data_bytes) : '...'}</strong>
          <span className="an-tile-sub">{stats ? `Database ${fmtBytes(stats.db_bytes)}` : ''}</span>
        </div>
      </div>

      <section className="block" aria-labelledby="an-trends-h">
        <div className="block-head">
          <h2 id="an-trends-h">Trends</h2>
          <div className="block-tools">
            <p>Cost</p>
            <div className="seg-mini" role="group" aria-label="Cost lines">
              <button type="button" className="seg-mini-btn" aria-pressed={costBy === 'total'} onClick={() => setCostBy('total')}>
                Total
              </button>
              <button type="button" className="seg-mini-btn" aria-pressed={costBy === 'project'} onClick={() => setCostBy('project')}>
                By project
              </button>
            </div>
          </div>
        </div>
        {samples == null ? (
          <div className="list">
            <p className="empty-line">Loading samples.</p>
          </div>
        ) : !anySamples ? (
          <div className="list">
            <p className="empty-line">Samples start once a session runs. One is taken each minute.</p>
          </div>
        ) : (
          <div className="an-trends">
            <div className="list an-card an-card--wide">
              <TrendChart
                title="Plan use"
                series={plan}
                since={since}
                now={now}
                range={range}
                format={pct}
                yMax={100}
                mark={{ v: PLAN_HIGH_AT, label: `${PLAN_HIGH_AT}%` }}
              />
            </div>
            <div className="list an-card">
              <TrendChart title="Cost added" series={cost} since={since} now={now} range={range} format={money} area />
            </div>
            <div className="list an-card">
              <TrendChart title="Memory" series={memory} since={since} now={now} range={range} format={bytes} nice={niceBytes} area />
            </div>
          </div>
        )}
      </section>

      <section className="block" aria-labelledby="an-sessions-h">
        <div className="block-head">
          <h2 id="an-sessions-h">Sessions</h2>
          <div className="block-tools">
            <p>Worked, waited and asked count the range. The rest are totals.</p>
            <button
              type="button"
              className="btn"
              disabled={targets.length === 0}
              title={targets.length === 0 ? 'No idle session needs compacting.' : undefined}
              onClick={() => setConfirm(true)}
            >
              Compact all recommended
            </button>
          </div>
        </div>
        <div className="list an-table-wrap">
          {sorted.length === 0 ? (
            <p className="empty-line">No sessions yet.</p>
          ) : (
            <table className="an-table">
              <thead>
                <tr>
                  <th>Session</th>
                  <th>Status</th>
                  {th('cost', 'Cost')}
                  {th('contextPct', 'Context')}
                  {th('memory', 'Memory')}
                  {th('disk', 'Disk')}
                  {th('lines', 'Lines')}
                  <th>Model</th>
                  {th('workedMs', 'Worked')}
                  {th('waitedMs', 'Waited')}
                  {th('asked', 'Asked')}
                  <th>Recommend</th>
                </tr>
              </thead>
              <tbody>
                {sorted.map((r) => (
                  <Row key={r.id} row={r} busy={!!sending[r.id]} onCompact={() => void compact([r.id])} />
                ))}
              </tbody>
            </table>
          )}
        </div>
      </section>

      {confirm && (
        <Dialog title="Compact recommended sessions" onClose={() => setConfirm(false)}>
          <p className="muted">
            Sends /compact to {targets.length === 1 ? 'this idle session' : `these ${targets.length} idle sessions`}. Claude Code
            summarizes the chat so far and keeps going from the summary.
          </p>
          <ul className="an-confirm-list">
            {targets.map((t) => (
              <li key={t.id}>
                <strong>{t.title}</strong> <span className="muted">{t.projectName}</span>
                <span className="faint"> context {t.contextPct == null ? '' : pct(t.contextPct)}</span>
              </li>
            ))}
          </ul>
          <div className="dialog-actions">
            <button type="button" className="btn" onClick={() => setConfirm(false)}>
              Cancel
            </button>
            <button
              type="button"
              className="btn btn--primary"
              onClick={() => {
                setConfirm(false);
                void compact(targets.map((t) => t.id));
              }}
            >
              Compact {targets.length}
            </button>
          </div>
        </Dialog>
      )}
    </div>
  );
}

function LimitTile({ label, limit, now }: { label: string; limit: PlanLimit | null; now: number }) {
  if (!limit) {
    return (
      <div className="an-tile">
        <span className="an-tile-label">{label}</span>
        <strong className="an-tile-value faint">None yet</strong>
        <span className="an-tile-sub">Claude plans only</span>
      </div>
    );
  }
  const v = Math.max(0, Math.min(100, limit.used_percentage));
  const resets = resetText(limit.resets_at, now);
  return (
    <div className="an-tile">
      <span className="an-tile-label">{label}</span>
      <strong className="an-tile-value">{pct(v)}</strong>
      <span className={`meter meter--${level(v)}`} role="meter" aria-valuenow={Math.round(v)} aria-valuemin={0} aria-valuemax={100} aria-label={label}>
        <span style={{ width: `${v}%` }} />
      </span>
      <span className="an-tile-sub">{resets ? `Resets ${resets}` : ''}</span>
    </div>
  );
}

const ctxLevel = (v: number) => (v >= 85 ? 'high' : v >= 70 ? 'warn' : 'ok');

function Row({ row: r, busy, onCompact }: { row: SessionRow; busy: boolean; onCompact: () => void }) {
  const go = () => {
    window.location.hash = `#session/${r.id}`;
  };
  const can = canCompact(r.status);
  return (
    <tr className="an-row" onClick={go}>
      <td>
        <div className="an-who">
          <a href={`#session/${r.id}`} onClick={(e) => e.stopPropagation()}>
            <strong>{r.title}</strong>
          </a>
          <span className="muted">{r.projectName}</span>
        </div>
      </td>
      <td>
        <span className="an-status">
          <Lamp status={r.status} />
          {STATUS_LABEL[r.status]}
        </span>
      </td>
      <td className="num" title={r.costInRange != null ? `${fmtCost(r.costInRange)} in this range` : undefined}>
        {r.cost == null ? '' : fmtCost(r.cost)}
      </td>
      <td className="num">
        {r.contextPct != null && (
          <span className="an-ctx">
            <span className={`meter meter--${ctxLevel(r.contextPct)}`} aria-hidden="true">
              <span style={{ width: `${Math.min(100, r.contextPct)}%` }} />
            </span>
            {pct(r.contextPct)}
          </span>
        )}
      </td>
      <td className="num">{r.memory == null ? '' : fmtBytes(r.memory)}</td>
      <td className="num">{r.disk == null ? '' : fmtBytes(r.disk)}</td>
      <td className="num">
        {r.linesAdded != null && (
          <>
            <span className="an-add">+{r.linesAdded}</span> <span className="an-del">-{r.linesRemoved ?? 0}</span>
          </>
        )}
      </td>
      <td className="an-model">{r.model ?? ''}</td>
      <td className="num">{r.workedMs ? fmtDur(r.workedMs) : ''}</td>
      <td className="num">{r.waitedMs ? fmtDur(r.waitedMs) : ''}</td>
      <td className="num">{r.asked || ''}</td>
      <td className="an-rec-cell">
        <div className="an-recs">
          {r.recs.map((x) => (
            <span key={x.id} className="tag" title={x.detail}>
              {x.label}
            </span>
          ))}
          {isRunning(r.status) && (
            <button
              type="button"
              className="btn an-compact"
              disabled={!can.ok || busy}
              title={can.why ?? 'Send /compact to this session'}
              onClick={(e) => {
                e.stopPropagation();
                onCompact();
              }}
            >
              Compact
            </button>
          )}
        </div>
      </td>
    </tr>
  );
}

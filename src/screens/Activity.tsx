import { useEffect, useMemo, useState } from 'react';
import { eventText } from '../lib/eventText';
import { clock } from '../lib/format';
import { request } from '../lib/ipc';
import type { EventRecord } from '../lib/types';
import { useStore } from '../store/store';

const LIMIT = 200;

function dotClass(e: EventRecord): string {
  if (/fail|denied|blocked/.test(e.kind)) return 'dot--failed';
  if (e.kind.startsWith('approval')) return 'dot--needs';
  if (/done|finish/.test(e.kind)) return 'dot--done';
  if (e.actor === 'user') return 'dot--user';
  return '';
}

export function Activity() {
  const sessions = useStore((s) => s.sessions);
  const live = useStore((s) => s.events);
  const [sessionId, setSessionId] = useState<string>('');
  const [fetched, setFetched] = useState<EventRecord[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    setLoaded(false);
    request<EventRecord[]>('events_list', { session_id: sessionId || null, limit: LIMIT })
      .then((list) => {
        if (!current) return;
        setFetched(list);
        setFailed(null);
        setLoaded(true);
      })
      .catch((err: Error) => {
        if (!current) return;
        setFailed(err.message);
        setLoaded(true);
      });
    return () => {
      current = false;
    };
  }, [sessionId]);

  const events = useMemo(() => {
    const bySeq = new Map<number, EventRecord>();
    for (const e of fetched) bySeq.set(e.seq, e);
    for (const e of live) if (!sessionId || e.session_id === sessionId) bySeq.set(e.seq, e);
    return [...bySeq.values()].sort((a, b) => b.seq - a.seq).slice(0, LIMIT);
  }, [fetched, live, sessionId]);

  const sessionList = useMemo(() => Object.values(sessions).sort((a, b) => b.started_at - a.started_at), [sessions]);

  return (
    <div className="screen">
      <div className="screen-head">
        <div>
          <h1>Activity</h1>
          <p>Everything sessions and you did, newest first.</p>
        </div>
      </div>
      <div className="filters-row">
        <label className="muted" htmlFor="activity-session">
          Session
        </label>
        <select id="activity-session" className="btn" value={sessionId} onChange={(e) => setSessionId(e.target.value)}>
          <option value="">All sessions</option>
          {sessionList.map((s) => (
            <option key={s.id} value={s.id}>
              {s.title}
            </option>
          ))}
        </select>
      </div>
      <div className="list">
        {failed ? (
          <p className="empty-line">{failed}</p>
        ) : events.length === 0 ? (
          <p className="empty-line">{loaded ? 'No activity yet. Events show up here as sessions run.' : 'Loading activity.'}</p>
        ) : (
          <ol className="events" style={{ padding: '4px 16px' }}>
            {events.map((e) => {
              const title = e.session_id ? (sessions[e.session_id]?.title ?? 'Removed session') : 'No session';
                            return (
                <li className="event" key={e.seq}>
                  <time>{clock(e.ts)}</time>
                  <span className={`dot ${dotClass(e)}`} />
                  <span>
                    <strong>{eventText(e)}</strong> <span className="muted">{e.actor}</span> <span className="muted">{title}</span>
                  </span>
                </li>
              );
            })}
          </ol>
        )}
      </div>
    </div>
  );
}

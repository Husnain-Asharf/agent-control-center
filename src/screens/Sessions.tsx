import { useMemo } from 'react';
import { Icon } from '../components/Icon';
import { Lamp, STATUS_LABEL } from '../components/Lamp';
import { openNewSession } from '../dialogs/NewSessionDialog';
import { useStore } from '../store/store';

export function Sessions() {
  const sessions = useStore((s) => s.sessions);
  const projects = useStore((s) => s.projects);
  const list = useMemo(() => Object.values(sessions).sort((a, b) => b.started_at - a.started_at), [sessions]);

  return (
    <div className="screen">
      <div className="screen-head">
        <div>
          <h1>Sessions</h1>
          <p>
            {list.length} {list.length === 1 ? 'session' : 'sessions'}
          </p>
        </div>
        <button className="btn btn--primary" type="button" onClick={openNewSession}>
          <Icon name="plus" />
          New session<kbd className="kbd">N</kbd>
        </button>
      </div>
      <div className="list">
        {list.length === 0 ? (
          <p className="empty-line">No sessions yet. Start one and it gets its own worktree.</p>
        ) : (
          list.map((s) => (
            <a className="proj-row" key={s.id} href={`#session/${s.id}`}>
              <Lamp status={s.status} />
              <span className="who">
                <strong>{s.title}</strong>
                <span>
                  {projects[s.project_id]?.name ?? 'Unknown project'} · {s.branch}
                </span>
              </span>
              <span className="proj-state">{STATUS_LABEL[s.status]}</span>
              <span className="chev">
                <Icon name="chevron" />
              </span>
            </a>
          ))
        )}
      </div>
    </div>
  );
}

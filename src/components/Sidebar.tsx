import { useMemo, useState } from 'react';
import { sessionsByProject, useStore } from '../store/store';
import { useRoute, type RouteName } from '../lib/route';
import { openNewSession } from '../dialogs/NewSessionDialog';
import { Icon, type IconName } from './Icon';
import { Lamp, STATUS_LABEL } from './Lamp';

const NAV: { route: RouteName; hash: string; label: string; icon: IconName }[] = [
  { route: 'overview', hash: '#overview', label: 'Overview', icon: 'overview' },
  { route: 'sessions', hash: '#sessions', label: 'Sessions', icon: 'sessions' },
  { route: 'projects', hash: '#projects', label: 'Projects', icon: 'projects' },
  { route: 'approvals', hash: '#approvals', label: 'Approvals', icon: 'approvals' },
  { route: 'activity', hash: '#activity', label: 'Activity', icon: 'activity' },
  { route: 'settings', hash: '#settings', label: 'Settings', icon: 'settings' },
];

export function Sidebar() {
  const route = useRoute();
  const projects = useStore((s) => s.projects);
  const sessions = useStore((s) => s.sessions);
  const approvals = useStore((s) => s.approvals);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const groups = useMemo(() => sessionsByProject({ projects, sessions }), [projects, sessions]);
  const pending = Object.values(approvals).filter((a) => a.state === 'pending').length;

  const current = (r: RouteName) =>
    route.name === r || (r === 'sessions' && route.name === 'session') || (r === 'projects' && route.name === 'project');

  const count = (r: RouteName): { n: number; cls: string } | null => {
    if (r === 'sessions') return { n: Object.keys(sessions).length, cls: '' };
    if (r === 'projects') return { n: Object.keys(projects).length, cls: '' };
    if (r === 'approvals' && pending > 0) return { n: pending, cls: ' nav-count--needs' };
    return null;
  };

  return (
    <nav className="sidebar" aria-label="Main">
      <div className="brand">
        <svg className="brand-mark" viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="12" cy="12" r="8.6" />
          <circle className="core" cx="12" cy="12" r="3.4" />
        </svg>
        <span className="brand-name">Agent Control Center</span>
      </div>
      <div className="nav">
        {NAV.map((item) => {
          const c = count(item.route);
          return (
            <a
              key={item.route}
              className="nav-item"
              href={item.hash}
              aria-current={current(item.route) ? 'page' : undefined}
            >
              <Icon name={item.icon} />
              <span>{item.label}</span>
              {c && <span className={`nav-count${c.cls}`}>{c.n}</span>}
            </a>
          );
        })}
      </div>
      <div className="side-group">
        <div className="side-head">
          <span>Sessions by project</span>
          <button
            className="icon-btn icon-btn--sm"
            type="button"
            aria-label="New session"
            onClick={openNewSession}
          >
            <Icon name="plus" />
          </button>
        </div>
        {groups.length === 0 && (
          <a className="fleet-link" href="#projects">
            <span aria-hidden="true" />
            <span className="fleet-name">Add project</span>
          </a>
        )}
        <ul className="fleet-list">
          {groups.map(({ project, sessions: list }) => {
            const open = !collapsed[project.id];
            const curP = route.name === 'project' && route.param === project.id;
            return (
              <li className="proj-group" key={project.id}>
                <div className="proj-line">
                  <button
                    className="proj-toggle"
                    type="button"
                    aria-expanded={open}
                    aria-label={`${open ? 'Collapse' : 'Expand'} ${project.name}`}
                    onClick={() => setCollapsed((c) => ({ ...c, [project.id]: open }))}
                  >
                    <Icon name="chevron" />
                  </button>
                  <a
                    className="proj-link"
                    href={`#project/${project.id}`}
                    aria-current={curP ? 'page' : undefined}
                  >
                    <span className="proj-name">{project.name}</span>
                    <span className="proj-lamps" aria-hidden="true">
                      {list.map((s) => (
                        <Lamp key={s.id} status={s.status} />
                      ))}
                    </span>
                  </a>
                </div>
                {open && list.length > 0 && (
                  <ul className="fleet-list fleet-list--nested">
                    {list.map((s) => (
                      <li key={s.id}>
                        <a
                          className="fleet-link"
                          href={`#session/${s.id}`}
                          aria-current={route.name === 'session' && route.param === s.id ? 'page' : undefined}
                          aria-label={`${s.title}: ${STATUS_LABEL[s.status]}`}
                        >
                          <Lamp status={s.status} />
                          <span className="fleet-name">{s.title}</span>
                          <span className="fleet-time">{STATUS_LABEL[s.status]}</span>
                        </a>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            );
          })}
        </ul>
      </div>
    </nav>
  );
}

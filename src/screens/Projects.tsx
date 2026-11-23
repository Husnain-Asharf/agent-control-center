import { useMemo, useState } from 'react';
import { FleetLanes, RangeControl, byRank, useNow, type Range } from '../components/FleetLanes';
import { Icon } from '../components/Icon';
import { Lamp } from '../components/Lamp';
import { AddProjectDialog } from '../dialogs/AddProjectDialog';
import { Dialog } from '../components/Dialog';
import { Coordination } from '../components/Coordination';
import { openNewSession } from '../dialogs/NewSessionDialog';
import { monogram } from '../lib/format';
import { needsYouForProject } from '../lib/needs';
import { go, useRoute } from '../lib/route';
import { request } from '../lib/ipc';
import { errorText } from '../lib/errorText';
import { sessionsByProject, useStore } from '../store/store';

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export function Projects() {
  const projects = useStore((s) => s.projects);
  const sessions = useStore((s) => s.sessions);
  const approvals = useStore((s) => s.approvals);
  const groups = useMemo(() => sessionsByProject({ projects, sessions }), [projects, sessions]);
  const [adding, setAdding] = useState(false);
  const LIVE = ['starting', 'working', 'needs_you', 'idle'];
  const live = Object.values(sessions).filter((s) => LIVE.includes(s.status)).length;

  return (
    <div className="screen">
      <div className="screen-head">
        <div>
          <h1>Projects</h1>
          <p>
            {plural(groups.length, 'repository', 'repositories')} with {plural(live, 'live session', 'live sessions')}.
            Sessions work in the project folder and ask before they change the base branch.
          </p>
        </div>
        <button className="btn btn--primary" type="button" onClick={() => setAdding(true)}>
          <Icon name="plus" />
          Add project
        </button>
      </div>
      {groups.length === 0 ? (
        <div className="list">
          <p className="empty-line">No projects yet. Add a git repository to start a session in it.</p>
        </div>
      ) : (
        <div className="list">
          {groups.map(({ project, sessions: list }) => {
            const needs = needsYouForProject(project.id, sessions, approvals);
            return (
              <a className="proj-row" key={project.id} href={`#project/${project.id}`}>
                <span className="monogram monogram--sm">{monogram(project.name)}</span>
                <span className="who">
                  <strong>{project.name}</strong>
                  <span>{project.path}</span>
                </span>
                <span className="proj-lamps" aria-label={plural(list.length, 'session', 'sessions')}>
                  {list.length === 0 ? (
                    <span className="faint">No sessions</span>
                  ) : (
                    list.map((s) => <Lamp key={s.id} status={s.status} />)
                  )}
                </span>
                <span className={`proj-state${needs ? ' proj-state--needs' : ''}`}>
                  {needs ? `${needs} ${needs === 1 ? 'needs' : 'need'} you` : 'All clear'}
                </span>
                <span className="proj-meta">{plural(list.length, 'session', 'sessions')}</span>
                <span className="chev">
                  <Icon name="chevron" />
                </span>
              </a>
            );
          })}
        </div>
      )}
      {adding && <AddProjectDialog onClose={() => setAdding(false)} />}
    </div>
  );
}

export function ProjectPage() {
  const { param } = useRoute();
  const pushToast = useStore((s) => s.pushToast);
  const [removing, setRemoving] = useState(false);
  const [base, setBase] = useState<string | null>(null);
  const project = useStore((s) => (param ? s.projects[param] : undefined));
  const projects = useStore((s) => s.projects);
  const sessions = useStore((s) => s.sessions);
  const [range, setRange] = useState<Range>(60);
  const now = useNow();
  const list = useMemo(
    () =>
      Object.values(sessions)
        .filter((s) => s.project_id === param)
        .sort(byRank),
    [sessions, param],
  );

  if (!project) {
    return (
      <div className="screen">
        <nav className="crumbs" aria-label="Breadcrumb">
          <a href="#projects">Projects</a>
        </nav>
        <div className="empty">
          <h1>Project not found</h1>
          <p>This project is not in the app. Go back to Projects.</p>
        </div>
      </div>
    );
  }

  return (
    <div className="screen">
      <nav className="crumbs" aria-label="Breadcrumb">
        <a href="#projects">Projects</a>
        <span>/</span>
        <span>{project.name}</span>
      </nav>
      <div className="proj-head">
        <span className="monogram">{monogram(project.name)}</span>
        <div>
          <h1>{project.name}</h1>
          <p className="muted">
            <code>{project.path}</code>
            {project.host && <span>{project.host}</span>}
            <span>
              Base branch <code>{project.base_branch}</code>{' '}
              <button className="link" type="button" onClick={() => setBase(project.base_branch)}>
                Change
              </button>
            </span>
          </p>
        </div>
        <div className="head-actions">
          <button className="btn btn--ghost" type="button" onClick={() => setRemoving(true)}>
            Remove project
          </button>
          <button className="btn btn--primary" type="button" onClick={openNewSession}>
            <Icon name="plus" />
            New session
            <kbd className="kbd">N</kbd>
          </button>
        </div>
      </div>
      {base !== null && (
        <Dialog
          title="Base branch"
          onClose={() => setBase(null)}
          onSubmit={() =>
            request('project_set_base', { project_id: project.id, base_branch: base })
              .then(() => setBase(null))
              .catch((e: unknown) => pushToast({ msg: errorText(e), kind: 'failed' }))
          }
        >
          <p className="muted">
            Sessions in the project folder ask before they change this branch. The remote's default branch counts too.
          </p>
          <label className="field">
            <span>Branch</span>
            <input className="field-input" value={base} data-autofocus onChange={(e) => setBase(e.target.value)} />
          </label>
          <div className="dialog-actions">
            <button type="button" className="btn btn--ghost" onClick={() => setBase(null)}>
              Cancel
            </button>
            <button type="submit" className="btn btn--primary" disabled={base.trim() === ''}>
              Save
            </button>
          </div>
        </Dialog>
      )}
      {removing && (
        <Dialog
          title="Remove project"
          onClose={() => setRemoving(false)}
          onSubmit={() =>
            request('project_remove', { project_id: project.id })
              .then(() => go('#projects'))
              .catch((e: unknown) => {
                setRemoving(false);
                pushToast({ msg: errorText(e), kind: 'failed' });
              })
          }
        >
          <p className="muted">
            {project.name} and its sessions leave the app. The folder and its git history stay as they are. Stop its
            running sessions first.
          </p>
          <div className="dialog-actions">
            <button type="button" className="btn btn--ghost" onClick={() => setRemoving(false)}>
              Cancel
            </button>
            <button type="submit" className="btn btn--danger">
              Remove
            </button>
          </div>
        </Dialog>
      )}
      {list.length > 0 && (
        <section className="block" aria-labelledby="coord-h">
          <div className="block-head">
            <h2 id="coord-h">Coordination</h2>
            <p>Who works where, the files each changed in the last 2 hours, and where they meet</p>
          </div>
          <Coordination project={project} sessions={list} now={now} />
        </section>
      )}
      <section className="block" aria-labelledby="live-h">
        <div className="block-head">
          <h2 id="live-h">Sessions</h2>
          <RangeControl range={range} onChange={setRange} />
        </div>
        {list.length === 0 ? (
          <div className="list">
            <p className="empty-line">No sessions yet. Start one here and it works in the project folder.</p>
          </div>
        ) : (
          <FleetLanes sessions={list} projects={projects} range={range} now={now} />
        )}
      </section>
    </div>
  );
}

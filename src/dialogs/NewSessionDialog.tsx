import { useEffect, useMemo, useState } from 'react';
import { Dialog } from '../components/Dialog';
import { IpcError, request } from '../lib/ipc';
import { go, useRoute } from '../lib/route';
import type { Session } from '../lib/types';
import { useStore } from '../store/store';

const OPEN_EVENT = 'acc:new-session';

/** Ask the app shell to open the New session dialog. */
export function openNewSession(): void {
  window.dispatchEvent(new Event(OPEN_EVENT));
}

/** Calls `cb` whenever something asks for the dialog. */
export function onOpenNewSession(cb: () => void): () => void {
  window.addEventListener(OPEN_EVENT, cb);
  return () => window.removeEventListener(OPEN_EVENT, cb);
}

export function newSessionErrorText(e: unknown): string {
  if (e instanceof IpcError) return e.message;
  return e instanceof Error ? e.message : String(e);
}

export function NewSessionDialog({ onClose }: { onClose: () => void }) {
  const route = useRoute();
  const projectsById = useStore((s) => s.projects);
  const projects = useMemo(
    () => Object.values(projectsById).sort((a, b) => a.added_at - b.added_at || a.name.localeCompare(b.name)),
    [projectsById],
  );
  const preset = route.name === 'project' ? route.param : null;
  const [projectId, setProjectId] = useState(preset ?? '');
  const [task, setTask] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Fall back to the first project when nothing valid is selected.
  useEffect(() => {
    if (projects.length > 0 && !projects.some((p) => p.id === projectId)) setProjectId(projects[0].id);
  }, [projects, projectId]);

  const canStart = !busy && task.trim() !== '' && projectId !== '';

  const submit = async () => {
    if (!canStart) return;
    setBusy(true);
    setError(null);
    try {
      const session = await request<Session>('session_start', {
        project_id: projectId,
        agent: 'claude',
        task: task.trim(),
      });
      onClose();
      go(`#session/${session.id}`);
    } catch (e) {
      setError(newSessionErrorText(e));
      setBusy(false);
    }
  };

  return (
    <Dialog title="New session" onClose={onClose} onSubmit={submit}>
      <p className="muted">The session starts in its own worktree, so it cannot touch other sessions' files.</p>
      <div className="field">
        <span>Agent</span>
        <input className="field-input" value="Claude Code" readOnly aria-label="Agent" tabIndex={-1} />
      </div>
      <label className="field">
        <span>Project</span>
        <select name="project" value={projectId} onChange={(e) => setProjectId(e.target.value)} required>
          {projects.length === 0 && <option value="">No projects yet</option>}
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>Task</span>
        <textarea
          name="task"
          rows={3}
          required
          data-autofocus
          value={task}
          onChange={(e) => setTask(e.target.value)}
          placeholder="For example: add pagination to the orders endpoint"
        />
      </label>
      <p className="field-note">Starts on a new branch named after the task.</p>
      {error && (
        <p className="msg-flag" role="alert">
          {error}
        </p>
      )}
      <div className="dialog-actions">
        <button type="button" className="btn btn--ghost" onClick={onClose}>
          Cancel
        </button>
        <button type="submit" className="btn btn--primary" disabled={!canStart}>
          Start session
        </button>
      </div>
    </Dialog>
  );
}

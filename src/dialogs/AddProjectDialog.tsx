import { useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { Dialog } from '../components/Dialog';
import { IpcError, request } from '../lib/ipc';
import { go } from '../lib/route';
import type { Project } from '../lib/types';

export function addProjectErrorText(e: unknown): string {
  if (e instanceof IpcError) {
    return e.code === 'not_a_repo' ? 'That folder is not a git repository.' : e.message;
  }
  return e instanceof Error ? e.message : String(e);
}

export function AddProjectDialog({ onClose }: { onClose: () => void }) {
  const [path, setPath] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const choose = async () => {
    try {
      const picked = await open({ directory: true });
      if (typeof picked === 'string') {
        setPath(picked);
        setError(null);
      }
    } catch (e) {
      setError(addProjectErrorText(e));
    }
  };

  const submit = async () => {
    const value = path.trim();
    if (!value || busy) return;
    setBusy(true);
    setError(null);
    try {
      const project = await request<Project>('project_add', { path: value });
      onClose();
      go(`#project/${project.id}`);
    } catch (e) {
      setError(addProjectErrorText(e));
      setBusy(false);
    }
  };

  return (
    <Dialog title="Add a project" onClose={onClose} onSubmit={submit} wide>
      <p className="muted">Any git repository works. Each session in it gets its own worktree and branch.</p>
      <div className="field">
        <span>Folder</span>
        <div style={{ display: 'flex', gap: 8 }}>
          <input
            className="field-input"
            name="path"
            value={path}
            onChange={(e) => setPath(e.target.value)}
            required
            autoComplete="off"
            spellCheck={false}
            data-autofocus
            aria-label="Folder"
            aria-invalid={error ? true : undefined}
            style={{ flex: 1, fontFamily: 'var(--font-code)' }}
            placeholder="~/code/payments-gateway"
          />
          <button className="btn" type="button" onClick={choose}>
            Choose…
          </button>
        </div>
        {error && (
          <p className="msg-flag" role="alert">
            {error}
          </p>
        )}
      </div>
      <div className="dialog-actions">
        <button type="button" className="btn btn--ghost" onClick={onClose}>
          Cancel
        </button>
        <button type="submit" className="btn btn--primary" disabled={busy || !path.trim()}>
          Add project
        </button>
      </div>
    </Dialog>
  );
}

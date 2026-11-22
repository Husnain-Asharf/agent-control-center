import type { Approval, Session } from './types';

/** Pending approvals plus failed sessions, scoped to one project. */
export function needsYouForProject(
  projectId: string,
  sessions: Record<string, Session>,
  approvals: Record<string, Approval>,
): number {
  const own = Object.values(sessions).filter((s) => s.project_id === projectId);
  const ids = new Set(own.map((s) => s.id));
  const pending = Object.values(approvals).filter((a) => a.state === 'pending' && ids.has(a.session_id)).length;
  return pending + own.filter((s) => s.status === 'failed').length;
}

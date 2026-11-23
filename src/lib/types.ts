// Mirrors crates/acc-protocol/src/lib.rs. Timestamps are ms since epoch.

export type SessionStatus =
  | 'starting'
  | 'working'
  | 'needs_you'
  | 'idle'
  | 'done'
  | 'failed'
  | 'stopped';

export interface StatusChange {
  status: SessionStatus;
  at: number;
}

export interface Project {
  id: string;
  name: string;
  path: string;
  host: string | null;
  base_branch: string;
  added_at: number;
}

/** `project`: in the project folder, on its current branch. `worktree`: in its own worktree and branch. */
export type SessionMode = 'project' | 'worktree';

export interface Session {
  id: string;
  project_id: string;
  agent: string;
  title: string;
  /** The folder's current branch (project mode) or the session's own branch. */
  branch: string;
  /** The folder the session works in: the project folder or its worktree. */
  worktree: string;
  /** Missing from an older daemon: read as `worktree`. */
  mode?: SessionMode;
  /** The user let this session change files on the base branch, so it is not asked again. */
  stay_on_base?: boolean;
  status: SessionStatus;
  status_since: number;
  started_at: number;
  ended_at: number | null;
  exit_code: number | null;
  history: StatusChange[];
}

export type ApprovalState = 'pending' | 'allowed' | 'denied' | 'cancelled' | 'expired';

export interface Approval {
  id: string;
  session_id: string;
  tool: string;
  summary: string;
  input: unknown;
  created_at: number;
  state: ApprovalState;
  /** "user" or "policy" */
  decided_by: string | null;
  /** Missing from an older daemon: read as false. */
  protected?: boolean;
  /** What Always allow would cover. Null or missing means it can only be allowed once. */
  rule_preview?: string | null;
  /** `branch`: a session in the project folder wants to change files on the base branch. Missing means `permission`. */
  kind?: 'permission' | 'branch';
  /** For a branch question: the branch Make branch would create. */
  branch?: string | null;
}

export type Decision = 'allow_once' | 'allow_always' | 'deny' | 'make_branch' | 'stay_on_base';

export interface EventRecord {
  seq: number;
  ts: number;
  session_id: string | null;
  project_id: string | null;
  kind: string;
  actor: string;
  payload: unknown;
}

export interface Snapshot {
  projects: Project[];
  sessions: Session[];
  approvals: Approval[];
}

export type DaemonEvent =
  | { kind: 'session_updated'; data: Session }
  | { kind: 'session_output'; data: { session_id: string; data_b64: string; offset: number } }
  | { kind: 'approval_created'; data: Approval }
  | { kind: 'approval_resolved'; data: Approval }
  | { kind: 'project_added'; data: Project }
  | { kind: 'event_appended'; data: EventRecord };

export interface DaemonStatusPayload {
  connected: boolean;
  error?: string;
}

export type Theme = 'dark' | 'light';

export type ToastKind = SessionStatus | 'needs';

export interface Toast {
  id: number;
  msg: string;
  kind: ToastKind;
  /** Buttons shown in the toast. The first one is the primary action. */
  actions?: { label: string; run: () => void }[];
}

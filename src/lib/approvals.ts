import { useEffect, useState } from 'react';
import { isTypingTarget, type KeyLike } from './keys';
import { touchesProtectedPath } from './protected';
import { useStore, oldestPending } from '../store/store';
import { go } from './route';
import { request } from './ipc';
import { errorText } from './errorText';
import type { Approval, Decision } from './types';

const EDIT_TOOLS = ['Edit', 'MultiEdit', 'Write', 'NotebookEdit'];
/** The shell tools: Bash, and PowerShell, which Claude Code also has on Windows. */
const SHELL_TOOLS = ['Bash', 'PowerShell'];

/** The headline for a request, by tool. */
export function wantsText(tool: string): string {
  if (SHELL_TOOLS.includes(tool)) return 'Wants to run a command';
  if (EDIT_TOOLS.includes(tool)) return 'Wants to edit a file';
  return `Wants to use ${tool}`;
}

/** The short form used in the dock and toasts: `run`, `edit` or `use <tool>`. */
export function wantsVerb(tool: string): string {
  if (SHELL_TOOLS.includes(tool)) return 'run';
  if (EDIT_TOOLS.includes(tool)) return 'edit';
  return `use ${tool}`;
}

export type ApprovalKey = Decision | 'next' | 'prev';

/** Maps a key press on the Approvals screen: A, shift+A, D, J and K. Null for anything else. */
export function approvalKey(e: KeyLike): ApprovalKey | null {
  if (e.metaKey || e.ctrlKey || e.altKey || e.defaultPrevented) return null;
  if (isTypingTarget(e.target)) return null;
  switch (e.key.toLowerCase()) {
    case 'a':
      if (e.repeat) return null;
      return e.shiftKey ? 'allow_always' : 'allow_once';
    case 'd':
      return e.repeat ? null : 'deny';
    case 'j':
      return 'next';
    case 'k':
      return 'prev';
    default:
      return null;
  }
}

export const DECISION_COOLDOWN_MS = 400;

/** Decision keys wait briefly after the selection changes, so a late key press cannot hit a new item. */
export function decisionKeysReady(now: number, selectedSince: number, cooldownMs = DECISION_COOLDOWN_MS): boolean {
  return now - selectedSince >= cooldownMs;
}

export const PROTECTED_NOTE = 'Protected files can never be always allowed';
export const ONCE_ONLY_NOTE = 'This request can only be allowed once';

type ApprovalFlags = Pick<Approval, 'summary' | 'protected' | 'rule_preview'>;

/** The daemon's `protected` flag. The summary check runs only when an older daemon leaves it out. */
export function isProtectedApproval(a: Pick<Approval, 'summary' | 'protected'>): boolean {
  return a.protected ?? touchesProtectedPath(a.summary);
}

export interface AlwaysAllowState {
  enabled: boolean;
  /** What the rule would cover. */
  preview: string | null;
  /** Why the button is off. */
  note: string | null;
}

/** Whether Always allow is offered, and what to say about it. Every surface asks this. */
export function alwaysAllowState(a: ApprovalFlags): AlwaysAllowState {
  const preview = a.rule_preview ?? null;
  if (preview) return { enabled: true, preview, note: null };
  return { enabled: false, preview: null, note: isProtectedApproval(a) ? PROTECTED_NOTE : ONCE_ONLY_NOTE };
}

/** Decision keys do nothing while a dialog is open. Moving the selection still works. */
export function decisionKeyAllowed(k: ApprovalKey, dialogOpen: boolean): boolean {
  if (k === 'next' || k === 'prev') return true;
  return !dialogOpen;
}

/** Sends a decision. `already_resolved` is not an error: the item has already left the inbox. */
const inFlight = new Set<string>();

/** A second decide for the same approval while the first is pending does nothing. */
export async function decide(approvalId: string, decision: Decision): Promise<void> {
  if (inFlight.has(approvalId)) return;
  const known = useStore.getState().approvals[approvalId];
  if (decision === 'allow_always' && known) {
    const state = alwaysAllowState(known);
    if (!state.enabled) {
      useStore.getState().pushToast({ msg: `${state.note}.`, kind: 'needs' });
      return;
    }
  }
  inFlight.add(approvalId);
  try {
    await request('approval_decide', { approval_id: approvalId, decision });
  } catch (e) {
    useStore.getState().pushToast({ msg: errorText(e), kind: 'failed' });
  } finally {
    inFlight.delete(approvalId);
  }
}

/** Opens the oldest pending approval. Does nothing when none is waiting. */
export function jumpToOldest(): void {
  const a = oldestPending(useStore.getState());
  if (a) go(`#approvals/${a.id}`);
}

/** Current time, refreshed every second while mounted. */
export function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  return now;
}

import { STATUS_LABEL } from '../components/Lamp';
import type { EventRecord, SessionStatus } from './types';

const str = (v: unknown): string | null => (typeof v === 'string' && v !== '' ? v : null);

/** `Bash: npm test`, or just the tool when the summary adds nothing. */
function toolLine(p: Record<string, unknown>): string {
  const tool = str(p.tool);
  const summary = str(p.summary);
  if (tool && summary && summary !== tool) return `${tool}: ${summary}`;
  return summary ?? tool ?? 'a tool';
}

function statusLabel(v: unknown): string {
  const s = str(v);
  return s ? (STATUS_LABEL[s as SessionStatus] ?? s) : 'unknown';
}

function decidedText(p: Record<string, unknown>, actor: string): string {
  const line = toolLine(p);
  const rule = p.rule && typeof p.rule === 'object' ? (p.rule as Record<string, unknown>) : null;
  const prefix = rule ? str(rule.prefix) : null;
  if (actor === 'policy') return prefix ? `Allowed by a saved rule (${prefix}): ${line}` : `Allowed by a saved rule: ${line}`;
  const onceOnly = str(p.once_only);
  if (onceOnly) return `Allowed once, no rule added (${onceOnly}): ${line}`;
  switch (p.decision) {
    case 'allow_once':
      return `Allowed once: ${line}`;
    case 'allow_always': {
      if (prefix) return `Always allowed ${prefix}: ${line}`;
      const tool = rule ? str(rule.tool) : null;
      return tool ? `Always allowed ${tool} requests: ${line}` : `Always allowed: ${line}`;
    }
    case 'deny':
      return `Denied: ${line}`;
    case 'make_branch':
      return `Made a branch first: ${line}`;
    case 'stay_on_base':
      return `Allowed on the base branch: ${line}`;
    default:
      return `Decided: ${line}`;
  }
}

/**
 * One line of text for an event, matched to the payloads accd writes
 * (daemon.rs). Unknown kinds and malformed payloads fall back to the kind.
 */
export function eventText(ev: Pick<EventRecord, 'kind' | 'actor' | 'payload'>): string {
  const p: Record<string, unknown> =
    ev.payload && typeof ev.payload === 'object' && !Array.isArray(ev.payload) ? (ev.payload as Record<string, unknown>) : {};
  switch (ev.kind) {
    case 'project.added':
      return `Added project ${str(p.name) ?? ''}`.trim();
    case 'session.started': {
      const branch = str(p.branch);
      const verb = p.resume === true ? 'Resumed' : 'Started';
      return branch ? `${verb} on branch ${branch}` : verb;
    }
    case 'session.status':
      return `Now ${statusLabel(p.status)}`;
    case 'session.exited':
      return typeof p.exit_code === 'number'
        ? `Exited with code ${p.exit_code}`
        : `Ended without an exit code (${statusLabel(p.status)})`;
    case 'approval.requested':
      return p.kind === 'branch' ? `Asked whether to branch before ${toolLine(p)}` : `Asked to use ${toolLine(p)}`;
    case 'branch.made':
      return `Made branch ${str(p.branch) ?? ''}`.trim();
    case 'approval.decided':
      return decidedText(p, ev.actor);
    case 'approval.cancelled':
      return p.reason === 'branch made'
        ? `Settled by branch ${str(p.branch) ?? ''}: ${toolLine(p)}`
        : `Request cancelled: ${toolLine(p)}`;
    case 'approval.expired':
      return `Request expired: ${toolLine(p)}`;
    case 'tool.used':
      return `Used ${toolLine(p)}`;
    default:
      return ev.kind;
  }
}

import type { SessionStatus } from '../lib/types';

export const STATUS_LABEL: Record<SessionStatus, string> = {
  starting: 'Starting',
  working: 'Working',
  needs_you: 'Needs you',
  idle: 'Idle',
  done: 'Done',
  failed: 'Failed',
  stopped: 'Stopped',
};

/** CSS suffix for the lamp class. The mockup calls needs_you "needs". */
export const lampClass = (status: SessionStatus): string =>
  `lamp lamp--${status === 'needs_you' ? 'needs' : status}`;

export function Lamp({ status }: { status: SessionStatus }) {
  return <span className={lampClass(status)} />;
}

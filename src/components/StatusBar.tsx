import { daemonStatusText } from '../lib/daemonStatus';
import { isMac, keyHints } from '../lib/platform';
import { useStore } from '../store/store';

export function StatusBar() {
  const connected = useStore((s) => s.connected);
  const daemonError = useStore((s) => s.daemonError);
  const nSessions = useStore((s) => Object.keys(s.sessions).length);
  const nProjects = useStore((s) => Object.keys(s.projects).length);
  const keys = keyHints(isMac);
  return (
    <footer className="statusbar">
      <span title={daemonError ?? undefined}>{daemonStatusText(connected, daemonError)}</span>
      <span>
        {nSessions} {nSessions === 1 ? 'session' : 'sessions'} in {nProjects}{' '}
        {nProjects === 1 ? 'project' : 'projects'}
      </span>
      <span className="spacer" />
      <span>
        <kbd className="kbd">{keys.palette}</kbd> Commands
      </span>
      {keys.nextNeeds && (
        <span>
          <kbd className="kbd">{keys.nextNeeds}</kbd> Next that needs you
        </span>
      )}
    </footer>
  );
}

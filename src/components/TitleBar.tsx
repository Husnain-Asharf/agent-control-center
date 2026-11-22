import { needsYouCount, useStore } from '../store/store';
import { go } from '../lib/route';
import { jumpToOldest } from '../lib/approvals';
import { openPalette } from '../lib/palette';
import { isMac, keyHints, platform } from '../lib/platform';
import { Icon } from './Icon';

/** On macOS the native traffic lights sit in the first 80px. On Windows and Linux the native title bar is above the webview. */
const TRAFFIC_LIGHTS_INSET = 80;

export function TitleBar() {
  const theme = useStore((s) => s.theme);
  const toggleTheme = useStore((s) => s.toggleTheme);
  const n = useStore((s) => needsYouCount(s));
  const label = n === 0 ? 'Nothing needs you' : n === 1 ? '1 needs you' : `${n} need you`;
  const themeLabel = theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme';
  const keys = keyHints(platform);

  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="traffic" data-tauri-drag-region style={isMac ? { paddingLeft: TRAFFIC_LIGHTS_INSET } : undefined} />
      <button className="search" type="button" data-action="palette" aria-label="Search or run a command" onClick={openPalette}>
        <Icon name="search" />
        <span>Search or run a command</span>
        <kbd className="kbd">{keys.palette}</kbd>
      </button>
      <div className="titlebar-right">
        <button className="icon-btn bell" type="button" aria-label="Notifications" onClick={() => go('#approvals')}>
          <Icon name="bell" />
          {n > 0 && <span className="bell-count">{n}</span>}
        </button>
        <button
          className={`caution${n > 0 ? ' is-lit' : ''}`}
          type="button"
          aria-label={n ? `${n} need you. Jump to the next one.` : 'Nothing needs you'}
          onClick={jumpToOldest}
        >
          <span className="lamp" />
          <span>{label}</span>
          {keys.nextNeeds && <kbd className="kbd">{keys.nextNeeds}</kbd>}
        </button>
        <button className="icon-btn" type="button" aria-label={themeLabel} onClick={toggleTheme}>
          <Icon name={theme === 'dark' ? 'sun' : 'moon'} />
        </button>
      </div>
    </header>
  );
}

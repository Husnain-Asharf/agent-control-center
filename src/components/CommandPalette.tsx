import { useEffect, useMemo, useRef, useState } from 'react';
import { go } from '../lib/route';
import { soundActionLabel } from '../lib/palette';
import { useStore } from '../store/store';
import { Icon, type IconName } from './Icon';
import { Lamp } from './Lamp';
import type { SessionStatus } from '../lib/types';

interface Item {
  group: string;
  label: string;
  hint?: string;
  icon?: IconName;
  status?: SessionStatus;
  keys?: string;
  run: () => void;
}

interface Props {
  onClose: () => void;
  /** Shown only when given. */
  onNewSession?: () => void;
  onAddProject: () => void;
}

const SCREENS: [string, IconName, string, string][] = [
  ['Overview', 'overview', '#overview', 'G O'],
  ['Sessions', 'sessions', '#sessions', 'G S'],
  ['Approvals', 'approvals', '#approvals', 'G I'],
  ['Projects', 'projects', '#projects', 'G P'],
  ['Activity', 'activity', '#activity', 'G A'],
];

export function CommandPalette({ onClose, onNewSession, onAddProject }: Props) {
  const sessions = useStore((s) => s.sessions);
  const projects = useStore((s) => s.projects);
  const theme = useStore((s) => s.theme);
  const toggleTheme = useStore((s) => s.toggleTheme);
  const soundEnabled = useStore((s) => s.soundEnabled);
  const toggleSound = useStore((s) => s.toggleSound);
  const [query, setQuery] = useState('');
  const [sel, setSel] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  const items = useMemo(() => {
    const all: Item[] = SCREENS.map(([label, icon, hash, keys]) => ({
      group: 'Go to',
      label,
      icon,
      keys,
      run: () => go(hash),
    }));
    all.push({ group: 'Go to', label: 'Analytics', hint: 'Use, cost and memory', icon: 'analytics', run: () => go('#analytics') });
    all.push({ group: 'Go to', label: 'Settings', hint: 'Theme, text size, sound', icon: 'settings', run: () => go('#settings') });
    Object.values(sessions)
      .sort((a, b) => b.started_at - a.started_at)
      .forEach((s) =>
        all.push({
          group: 'Sessions',
          label: s.title,
          status: s.status,
          hint: projects[s.project_id]?.name,
          run: () => go(`#session/${s.id}`),
        }),
      );
    if (onNewSession) all.push({ group: 'Actions', label: 'New session', icon: 'plus', keys: 'N', run: onNewSession });
    all.push({ group: 'Actions', label: 'Add project', icon: 'projects', run: onAddProject });
    all.push({
      group: 'Actions',
      label: theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme',
      icon: theme === 'dark' ? 'sun' : 'moon',
      run: toggleTheme,
    });
    all.push({
      group: 'Actions',
      label: soundActionLabel(soundEnabled),
      icon: 'bell',
      run: toggleSound,
    });
    const q = query.trim().toLowerCase();
    return q ? all.filter((i) => `${i.label} ${i.hint ?? ''}`.toLowerCase().includes(q)) : all;
  }, [sessions, projects, theme, toggleTheme, soundEnabled, toggleSound, onNewSession, onAddProject, query]);

  const active = Math.min(sel, Math.max(0, items.length - 1));

  useEffect(() => {
    listRef.current?.querySelector(`#pal-${active}`)?.scrollIntoView?.({ block: 'nearest' });
  }, [active]);

  const run = (i: number) => {
    const item = items[i];
    if (!item) return;
    onClose();
    item.run();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSel(Math.min(items.length - 1, active + 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSel(Math.max(0, active - 1));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      run(active);
    } else if (e.key === 'Escape') {
      onClose();
    }
  };

  let lastGroup = '';
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="palette" role="dialog" aria-modal="true" aria-label="Command palette">
        <input
          className="pal-input"
          data-autofocus
          autoFocus
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setSel(0);
          }}
          onKeyDown={onKeyDown}
          placeholder="Search sessions, screens and commands"
          aria-label="Search sessions, screens and commands"
          role="combobox"
          aria-expanded="true"
          aria-controls="pal-list"
          aria-activedescendant={items.length ? `pal-${active}` : undefined}
          autoComplete="off"
          spellCheck={false}
        />
        <div className="pal-list" id="pal-list" role="listbox" ref={listRef}>
          {items.length === 0 && <p className="pal-empty">No matches. Try a session name or a screen.</p>}
          {items.map((it, idx) => {
            const header = it.group !== lastGroup ? <div className="pal-group">{it.group}</div> : null;
            lastGroup = it.group;
            return (
              <div key={`${it.group}:${idx}`} style={{ display: 'contents' }}>
                {header}
                <div
                  className="pal-item"
                  role="option"
                  id={`pal-${idx}`}
                  aria-selected={idx === active}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => run(idx)}
                >
                  {it.status ? <Lamp status={it.status} /> : <Icon name={it.icon!} />}
                  <span className="pal-label">
                    {it.label}
                    {it.hint && <span className="faint">{it.hint}</span>}
                  </span>
                  {it.keys ? (
                    <span className="pal-keys">
                      {it.keys.split(' ').map((k) => (
                        <kbd key={k} className="kbd">
                          {k}
                        </kbd>
                      ))}
                    </span>
                  ) : (
                    <span />
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

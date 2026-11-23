import { useEffect, useRef, useState } from 'react';
import { onDaemonEvent, onDaemonStatus, request } from './lib/ipc';
import { b64ToBytes } from './lib/b64';
import { publishOutput } from './lib/outputBus';
import { isTypingTarget, matchShortcut, shortcutAllowed } from './lib/keys';
import { onOpenPalette } from './lib/palette';
import type { Snapshot } from './lib/types';
import { useStore } from './store/store';
import { TitleBar } from './components/TitleBar';
import { Sidebar } from './components/Sidebar';
import { StatusBar } from './components/StatusBar';
import { Toasts } from './components/Toasts';
import { CommandPalette } from './components/CommandPalette';
import { AddProjectDialog } from './dialogs/AddProjectDialog';
import { Activity } from './screens/Activity';
import { Overview } from './screens/Overview';
import { ProjectPage, Projects } from './screens/Projects';
import { SessionView } from './screens/SessionView';
import { Sessions } from './screens/Sessions';
import { Approvals } from './screens/Approvals';
import { Settings } from './screens/Settings';
import { DEFAULT_TEXT_SCALE, stepTextScale } from './lib/appearance';
import { decide, isBranchQuestion, isProtectedApproval, jumpToOldest, wantsVerb } from './lib/approvals';
import { go, useRoute } from './lib/route';
import { NewSessionDialog, onOpenNewSession, openNewSession } from './dialogs/NewSessionDialog';
import { createReplayQueue } from './lib/snapshotQueue';
import { shouldChime } from './lib/chime';
import { bindAudioUnlock, playChime } from './lib/sound';
import { errorText } from './lib/errorText';
import type { DaemonEvent } from './lib/types';

const TITLES = {
  overview: 'Overview',
  sessions: 'Sessions',
  session: 'Session',
  approvals: 'Approvals',
  projects: 'Projects',
  project: 'Project',
  activity: 'Activity',
  settings: 'Settings',
} as const;

function Screen() {
  const route = useRoute();
  if (route.name === 'overview') return <Overview />;
  if (route.name === 'activity') return <Activity />;
  if (route.name === 'projects') return <Projects />;
  if (route.name === 'project') return <ProjectPage />;
  if (route.name === 'sessions') return <Sessions />;
  if (route.name === 'session') return <SessionView />;
  if (route.name === 'approvals') return <Approvals />;
  if (route.name === 'settings') return <Settings />;
  // Placeholders. Later tasks replace these with the real screens.
  return (
    <div style={{ padding: '32px 36px' }}>
      <h1>{TITLES[route.name]}</h1>
    </div>
  );
}

export default function App() {
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [addProjectOpen, setAddProjectOpen] = useState(false);
  const [newSession, setNewSession] = useState(false);
  const gTimer = useRef<number | undefined>(undefined);
  const gArmed = useRef(false);
  const paletteRef = useRef(false);
  paletteRef.current = paletteOpen;

  // The one global key handler. The Approvals screen keeps its own A, shift+A, D, J and K keys.
  // It runs in the capture phase and calls preventDefault on a key it uses, so a chord key
  // such as `g a` never also reaches the Approvals handler.
  useEffect(() => {
    const disarm = () => {
      gArmed.current = false;
      window.clearTimeout(gTimer.current);
    };
    const onKey = (e: KeyboardEvent) => {
      const hit = matchShortcut(e, gArmed.current);
      disarm();
      if (!hit) return;
      const ok = shortcutAllowed(hit.action, {
        typing: isTypingTarget(document.activeElement),
        dialogOpen: !!document.querySelector('[role="dialog"]:not(.palette)'),
        paletteOpen: paletteRef.current,
      });
      if (!ok) return;
      e.preventDefault();
      if (hit.action === 'palette') setPaletteOpen((o) => !o);
      else if (hit.action === 'new-session') openNewSession();
      else if (hit.action === 'next-needs') jumpToOldest();
      else if (hit.action === 'text-bigger' || hit.action === 'text-smaller') {
        const { textScale, setTextScale } = useStore.getState();
        setTextScale(stepTextScale(textScale, hit.action === 'text-bigger' ? 1 : -1));
      } else if (hit.action === 'text-reset') useStore.getState().setTextScale(DEFAULT_TEXT_SCALE);
      else if (hit.action === 'arm-g') {
        gArmed.current = true;
        gTimer.current = window.setTimeout(disarm, 1500);
      } else if (hit.action.startsWith('go:')) go(`#${hit.action.slice(3)}`);
    };
    window.addEventListener('keydown', onKey, true);
    const offNew = onOpenNewSession(() => setNewSession(true));
    const offPalette = onOpenPalette(() => setPaletteOpen(true));
    return () => {
      window.removeEventListener('keydown', onKey, true);
      offNew();
      offPalette();
      disarm();
    };
  }, []);

  useEffect(() => {
    const { applyEvent, applySnapshot, setConnected, pushToast } = useStore.getState();

    // Events that arrive while a snapshot request is in flight wait here and are applied after it.
    bindAudioUnlock();
    let lastChimeAt: number | null = null;
    const queue = createReplayQueue<DaemonEvent>();

    const handle = (e: DaemonEvent) => {
      if (e.kind === 'approval_created') {
        // Replayed events can repeat an approval the snapshot already had. Toast only for a new one.
        const isNew = !useStore.getState().approvals[e.data.id];
        applyEvent(e);
        const now = Date.now();
        if (shouldChime(useStore.getState().soundEnabled, isNew, now, lastChimeAt)) {
          lastChimeAt = now;
          playChime();
        }
        if (!isNew) return;
        const a = e.data;
        const title = useStore.getState().sessions[a.session_id]?.title ?? 'A session';
        pushToast({
          msg: `${title} wants to ${wantsVerb(a.tool)} ${a.summary}`,
          kind: 'needs',
          actions: [
            ...(isProtectedApproval(a) || isBranchQuestion(a) ? [] : [{ label: 'Allow once', run: () => void decide(a.id, 'allow_once') }]),
            { label: 'Open', run: () => go(`#approvals/${a.id}`) },
          ],
        });
      } else {
        applyEvent(e);
      }
    };

    const offEvent = onDaemonEvent((e) => {
      if (e.kind === 'session_output') {
        publishOutput(e.data.session_id, b64ToBytes(e.data.data_b64), e.data.offset);
      } else if (!queue.push(e)) {
        handle(e);
      }
    });

    const offStatus = onDaemonStatus((connected, error) => {
      setConnected(connected, error);
      if (!connected) return;
      const token = queue.begin();
      request<Snapshot>('snapshot')
        .then((snap) => {
          applySnapshot(snap);
          for (const e of queue.end(token)) handle(e);
        })
        .catch((err: unknown) => {
          for (const e of queue.end(token)) handle(e);
          pushToast({ msg: errorText(err), kind: 'failed' });
        });
    });

    return () => {
      offEvent();
      offStatus();
    };
  }, []);

  return (
    <>
      <div className="app">
        <TitleBar />
        <Sidebar />
        <main className="main" id="main" tabIndex={-1}>
          <OfflineBanner />
          <Screen />
        </main>
        <StatusBar />
      </div>
      {newSession && <NewSessionDialog onClose={() => setNewSession(false)} />}
      <div id="overlay-root" />
      <Toasts />
      {paletteOpen && (
        <CommandPalette
          onClose={() => setPaletteOpen(false)}
          onNewSession={openNewSession}
          onAddProject={() => setAddProjectOpen(true)}
        />
      )}
      {addProjectOpen && <AddProjectDialog onClose={() => setAddProjectOpen(false)} />}
    </>
  );
}

/** Says plainly when the daemon is not connected, so the screens are read as last known. */
function OfflineBanner() {
  const connected = useStore((s) => s.connected);
  const error = useStore((s) => s.daemonError);
  const epoch = useStore((s) => s.connEpoch);
  // Before the first connection the status bar says "Connecting", which is enough.
  if (connected || epoch === 0) return null;
  return (
    <p className="offline-banner" role="status">
      The daemon is not connected{error ? `: ${error}` : ''}. Sessions keep running, and the app reconnects by itself. What
      you see here is the last known state.
    </p>
  );
}

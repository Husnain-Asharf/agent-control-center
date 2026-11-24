import { useEffect, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Dialog } from '../components/Dialog';
import { request } from '../lib/ipc';
import { errorText } from '../lib/errorText';
import { TERM_FONT_SIZES, TEXT_SCALES, scaleLabel } from '../lib/appearance';
import {
  COLOR_LABEL,
  PRESET_IDS,
  PRESET_LABEL,
  closeStatePairs,
  lowContrastText,
  matchingPreset,
  type ColorKey,
} from '../lib/colors';
import { keyHints, platform } from '../lib/platform';
import {
  BLUR_MAX,
  BLUR_MIN,
  BLUR_STEP,
  OPACITY_MAX,
  OPACITY_MIN,
  OPACITY_START,
  OPACITY_STEP,
  STYLES,
  STYLE_LABEL,
  blurNote,
  opacityReads,
  styleNote,
  glassStatus,
  onGlassStatus,
  showsDesktop,
} from '../lib/style';
import type { Theme } from '../lib/types';
import { useStore } from '../store/store';

const THEMES: Theme[] = ['dark', 'light'];
const THEME_LABEL: Record<Theme, string> = { dark: 'Dark', light: 'Light' };

/** One button per option, like the time range on the Overview. */
function Choice<T extends string | number>({
  label,
  options,
  value,
  text,
  onChange,
}: {
  label: string;
  options: readonly T[];
  /** null presses no button. */
  value: T | null;
  text: (option: T) => string;
  onChange: (option: T) => void;
}) {
  return (
    <div className="seg-mini" role="group" aria-label={label}>
      {options.map((o) => (
        <button key={o} type="button" className="seg-mini-btn" aria-pressed={o === value} onClick={() => onChange(o)}>
          {text(o)}
        </button>
      ))}
    </div>
  );
}

function Switch({ label, on, onToggle }: { label: string; on: boolean; onToggle: () => void }) {
  return <button className="switch" type="button" role="switch" aria-checked={on} aria-label={label} onClick={onToggle} />;
}

/** A slider with its value beside it. */
function Slider({
  label,
  min,
  max,
  step,
  value,
  unit,
  disabled,
  onChange,
}: {
  label: string;
  min: number;
  max: number;
  step: number;
  value: number;
  unit: string;
  disabled?: boolean;
  onChange: (value: number) => void;
}) {
  return (
    <div className="slider">
      <input
        type="range"
        className="slider-input"
        aria-label={label}
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      <span className="slider-value">{`${value}${unit}`}</span>
    </div>
  );
}

/** Blur and opacity, shown while the style is Glass. */
function GlassRows() {
  const blur = useStore((s) => s.glassBlur);
  const setBlur = useStore((s) => s.setGlassBlur);
  const opacity = useStore((s) => s.glassOpacity);
  const setOpacity = useStore((s) => s.setGlassOpacity);
  // Re-read when the colors change, since the check reads them off <html>.
  useStore((s) => s.colors);
  useStore((s) => s.theme);
  const css = typeof document === 'undefined' ? null : getComputedStyle(document.documentElement);
  const reads = !css || opacityReads((n) => css.getPropertyValue(n), opacity);
  const auto = opacity === null;
  const [see, setSee] = useState(glassStatus());
  useEffect(() => onGlassStatus(() => setSee(glassStatus())), []);
  return (
    <>
      {showsDesktop(platform) && see.state === 'failed' && (
        <p className="msg-flag" role="alert">
          macOS did not turn on the see-through window ({see.why}), so Glass shows the app's own background. Restart the
          app once; if it stays, tell us which macOS version you have.
        </p>
      )}
      <Row title="Blur" note={blurNote(platform)}>
        <Slider label="Blur" min={BLUR_MIN} max={BLUR_MAX} step={BLUR_STEP} value={blur} unit="px" onChange={setBlur} />
      </Row>
      <Row
        title="Opacity"
        note={
          auto
            ? 'Auto keeps the panels as clear as they can be while text still reads. Move the slider to set it yourself.'
            : reads
              ? 'How solid the panels are. Popups stay a little more solid.'
              : 'Some text may be hard to read over a very light or dark desktop at this opacity.'
        }
      >
        <div className="slider-row">
          <label className="check">
            <input type="checkbox" checked={auto} onChange={(e) => setOpacity(e.target.checked ? null : OPACITY_START)} />
            Auto
          </label>
          <Slider
            label="Opacity"
            min={OPACITY_MIN}
            max={OPACITY_MAX}
            step={OPACITY_STEP}
            value={opacity ?? OPACITY_START}
            unit="%"
            onChange={setOpacity}
          />
        </div>
      </Row>
    </>
  );
}

function Row({ title, note, children }: { title: string; note?: ReactNode; children: ReactNode }) {
  return (
    <div className="rule-line">
      <div className="setting-text">
        <span>{title}</span>
        {note && <p>{note}</p>}
      </div>
      {children}
    </div>
  );
}

const COLOR_GROUPS: { title: string; keys: ColorKey[] }[] = [
  { title: 'Background', keys: ['chrome', 'panel', 'raised'] },
  { title: 'Text', keys: ['ink', 'ink-2'] },
  { title: 'States', keys: ['amber', 'red', 'green', 'cyan'] },
];

function ColorPick({ k, value, onChange }: { k: ColorKey; value: string; onChange: (hex: string) => void }) {
  return (
    <label className="color-pick">
      <input type="color" value={value} onChange={(e) => onChange(e.target.value)} />
      <span>{COLOR_LABEL[k]}</span>
    </label>
  );
}

function Colors() {
  const theme = useStore((s) => s.theme);
  const choice = useStore((s) => s.colors[s.theme]);
  const setColorPreset = useStore((s) => s.setColorPreset);
  const setColor = useStore((s) => s.setColor);
  const resetColors = useStore((s) => s.resetColors);
  const { colors } = choice;
  const preset = matchingPreset(theme, choice);
  const close = closeStatePairs(colors);
  const lowText = lowContrastText(colors);

  return (
    <section className="block" aria-labelledby="colors-h">
      <div className="block-head">
        <h2 id="colors-h">Colors</h2>
        <p>For the {THEME_LABEL[theme].toLowerCase()} theme. Each theme keeps its own.</p>
      </div>
      <div className="list">
        <Row title="Preset" note={preset ? undefined : 'Custom colors.'}>
          <Choice label="Preset" options={PRESET_IDS} value={preset} text={(p) => PRESET_LABEL[p]} onChange={setColorPreset} />
        </Row>
        {COLOR_GROUPS.map((g) => (
          <Row key={g.title} title={g.title} note={g.title === 'States' ? 'The app shows session state with these.' : undefined}>
            <div className="color-picks">
              {g.keys.map((k) => (
                <ColorPick key={k} k={k} value={colors[k]} onChange={(hex) => setColor(k, hex)} />
              ))}
            </div>
          </Row>
        ))}
        {(close.length > 0 || lowText.length > 0) && (
          <div className="rule-line color-warn" role="status">
            <div className="setting-text">
              {close.map(([a, b]) => (
                <p key={a + b}>
                  {COLOR_LABEL[a]} and {COLOR_LABEL[b]} look alike, so session states may be hard to tell apart.
                </p>
              ))}
              {lowText.map((k) => (
                <p key={k}>{COLOR_LABEL[k]} is hard to read on the panels.</p>
              ))}
            </div>
          </div>
        )}
        <Row title="Reset colors" note={`Back to the default ${THEME_LABEL[theme].toLowerCase()} colors.`}>
          <button className="btn" type="button" disabled={preset === 'default'} onClick={resetColors}>
            Reset
          </button>
        </Row>
      </div>
    </section>
  );
}

export function Settings() {
  const theme = useStore((s) => s.theme);
  const toggleTheme = useStore((s) => s.toggleTheme);
  const style = useStore((s) => s.style);
  const setStyle = useStore((s) => s.setStyle);
  const textScale = useStore((s) => s.textScale);
  const setTextScale = useStore((s) => s.setTextScale);
  const bolder = useStore((s) => s.bolder);
  const toggleBolder = useStore((s) => s.toggleBolder);
  const termFontSize = useStore((s) => s.termFontSize);
  const setTermFontSize = useStore((s) => s.setTermFontSize);
  const soundEnabled = useStore((s) => s.soundEnabled);
  const toggleSound = useStore((s) => s.toggleSound);
  const keys = keyHints(platform).textSize;

  return (
    <div className="screen screen--narrow">
      <div className="screen-head">
        <div>
          <h1>Settings</h1>
          <p>Saved on this computer.</p>
        </div>
      </div>
      <section className="block" aria-labelledby="appearance-h">
        <div className="block-head">
          <h2 id="appearance-h">Appearance</h2>
        </div>
        <div className="list">
          <Row title="Theme">
            <Choice
              label="Theme"
              options={THEMES}
              value={theme}
              text={(t) => THEME_LABEL[t]}
              onChange={(t) => {
                if (t !== theme) toggleTheme();
              }}
            />
          </Row>
          <Row title="Style" note={styleNote(platform)}>
            <Choice label="Style" options={STYLES} value={style} text={(v) => STYLE_LABEL[v]} onChange={setStyle} />
          </Row>
          {style === 'glass' && <GlassRows />}
          <Row
            title="Text size"
            note={
              <>
                Zooms the whole window. <kbd className="kbd">{keys.bigger}</kbd> and <kbd className="kbd">{keys.smaller}</kbd>{' '}
                change it, <kbd className="kbd">{keys.reset}</kbd> goes back to 100%.
              </>
            }
          >
            <Choice label="Text size" options={TEXT_SCALES} value={textScale} text={scaleLabel} onChange={setTextScale} />
          </Row>
          <Row title="Bolder text" note="Draws text one weight heavier.">
            <Switch label="Bolder text" on={bolder} onToggle={toggleBolder} />
          </Row>
          <Row title="Terminal text size" note="Font size of session terminals, in pixels.">
            <Choice
              label="Terminal text size"
              options={TERM_FONT_SIZES}
              value={termFontSize}
              text={String}
              onChange={setTermFontSize}
            />
          </Row>
        </div>
      </section>
      <Colors />
      <section className="block" aria-labelledby="sound-h">
        <div className="block-head">
          <h2 id="sound-h">Sound</h2>
        </div>
        <div className="list">
          <Row title="Approval chime" note="Plays when a session waits for your approval.">
            <Switch label="Approval chime" on={soundEnabled} onToggle={toggleSound} />
          </Row>
        </div>
      </section>
      <Restart />
    </div>
  );
}

function Restart() {
  const sessions = useStore((s) => s.sessions);
  const pushToast = useStore((s) => s.pushToast);
  const [asking, setAsking] = useState(false);
  const running = Object.values(sessions).filter((s) => ['starting', 'working', 'needs_you', 'idle'].includes(s.status)).length;
  const fail = (e: unknown) => pushToast({ msg: errorText(e), kind: 'failed' });
  return (
    <section className="block" aria-labelledby="restart-h">
      <div className="block-head">
        <h2 id="restart-h">Restart</h2>
      </div>
      <div className="list">
        <Row title="Restart the window" note="Reloads the screens. Sessions keep running.">
          <button className="btn" type="button" onClick={() => window.location.reload()}>
            Reload
          </button>
        </Row>
        <Row title="Restart the app" note="Quits and opens the app again. Sessions keep running in the daemon.">
          <button className="btn" type="button" onClick={() => invoke('restart_app').catch(fail)}>
            Restart app
          </button>
        </Row>
        <Row
          title="Restart the daemon"
          note="Stops every session and starts the daemon again. Use it after an update. Sessions come back with Restart."
        >
          <button className="btn btn--danger" type="button" onClick={() => setAsking(true)}>
            Restart daemon
          </button>
        </Row>
      </div>
      {asking && (
        <Dialog
          title="Restart the daemon"
          onClose={() => setAsking(false)}
          onSubmit={() => {
            setAsking(false);
            request('daemon_restart').catch(fail);
          }}
        >
          <p className="muted">
            {running === 0
              ? 'No session is running.'
              : `${running} running ${running === 1 ? 'session stops' : 'sessions stop'}. Each comes back with Restart, in the same conversation.`}
          </p>
          <div className="dialog-actions">
            <button type="button" className="btn btn--ghost" onClick={() => setAsking(false)}>
              Cancel
            </button>
            <button type="submit" className="btn btn--danger">
              Restart daemon
            </button>
          </div>
        </Dialog>
      )}
    </section>
  );
}

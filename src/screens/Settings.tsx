import type { ReactNode } from 'react';
import { TERM_FONT_SIZES, TEXT_SCALES, scaleLabel } from '../lib/appearance';
import { keyHints, platform } from '../lib/platform';
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
  value: T;
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

export function Settings() {
  const theme = useStore((s) => s.theme);
  const toggleTheme = useStore((s) => s.toggleTheme);
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
    </div>
  );
}

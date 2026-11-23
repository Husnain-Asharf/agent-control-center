import { describe, expect, it } from 'vitest';
import appCss from '../styles/app.css?raw';
import tokens from '../styles/tokens.css?raw';
import { colorDistance, presetColors, PRESET_IDS } from './colors';
import {
  DEFAULT_STYLE,
  GLASS_VARS,
  glassAlpha,
  glassVars,
  liftText,
  parseStyle,
  readsOver,
  showsDesktop,
  styleNote,
} from './style';

/** The variables of one theme block in tokens.css. */
function tokenVars(selector: string): Record<string, string> {
  const start = tokens.indexOf(selector);
  const block = tokens.slice(start, tokens.indexOf('}', start));
  return Object.fromEntries([...block.matchAll(/(--[\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
}
const DARK = tokenVars(':root, :root[data-theme="dark"]');
const LIGHT = tokenVars(':root[data-theme="light"]');
const reader = (vars: Record<string, string>) => (name: string) => vars[name] ?? '';

/** The alpha out of an rgba() string. */
const alphaOf = (v: string) => Number(v.match(/,([\d.]+)\)$/)?.[1]);
const hexOf = (v: string) => {
  const [r, g, b] = v.match(/\d+/g)!.map(Number);
  return `#${[r, g, b].map((c) => c.toString(16).padStart(2, '0')).join('')}`;
};

describe('saved style', () => {
  it.each(['solid', 'glass'])('reads %s', (saved) => {
    expect(parseStyle(saved)).toBe(saved);
  });
  it.each([null, '', 'Glass', 'frosted'])('falls back to solid for %j', (saved) => {
    expect(parseStyle(saved)).toBe('solid');
  });
  it('is solid by default', () => {
    expect(DEFAULT_STYLE).toBe('solid');
  });
});

describe('platform note', () => {
  it('shows the desktop only on macOS', () => {
    expect(showsDesktop('mac')).toBe(true);
    expect(showsDesktop('windows')).toBe(false);
    expect(showsDesktop('linux')).toBe(false);
  });
  it('says where the desktop does not show through', () => {
    expect(styleNote('mac')).toMatch(/desktop show through/);
    expect(styleNote('windows')).toMatch(/does not show through on Windows/);
    expect(styleNote('linux')).toMatch(/does not show through on Linux/);
  });
});

describe('glass contrast', () => {
  it('needs the text to read over black and white backdrops', () => {
    expect(readsOver('#13181c', 1, '#e8ecef')).toBe(true);
    // Almost clear: a white desktop washes the light text out.
    expect(readsOver('#13181c', 0.2, '#e8ecef')).toBe(false);
  });

  it('picks the lowest alpha that keeps AA', () => {
    const a = glassAlpha('#13181c', ['#e8ecef', '#a3aeb6'], 0.5);
    expect(a).toBeGreaterThan(0.5);
    expect(a).toBeLessThan(1);
    expect(readsOver('#13181c', a, '#a3aeb6')).toBe(true);
    expect(readsOver('#13181c', a - 0.01, '#a3aeb6')).toBe(false);
  });

  it('never goes below the floor', () => {
    expect(glassAlpha('#000000', ['#ffffff'], 0.9)).toBe(0.9);
  });

  it('stays opaque when the text does not read even then', () => {
    expect(glassAlpha('#13181c', ['#222222'], 0.5)).toBe(1);
  });

  it('lifts faint text toward the main text only as far as needed', () => {
    const surfaces = [{ bg: '#13181c', alpha: 0.82 }];
    const lifted = liftText('#7f8b94', '#e8ecef', surfaces);
    expect(lifted).not.toBe('#7f8b94');
    expect(readsOver('#13181c', 0.82, lifted)).toBe(true);
    // Text that already reads is left alone.
    expect(liftText('#e8ecef', '#e8ecef', surfaces)).toBe('#e8ecef');
  });
});

describe('glass colors', () => {
  it.each([
    ['dark', DARK],
    ['light', LIGHT],
  ])('makes every %s surface translucent and readable', (_, vars) => {
    const out = glassVars(reader(vars));
    for (const [name, from] of [
      ['--glass-chrome', '--chrome'],
      ['--glass-panel', '--panel'],
      ['--glass-raised', '--raised'],
    ]) {
      const v = out[name];
      expect(hexOf(v)).toBe(vars[from].toLowerCase());
      const alpha = alphaOf(v);
      expect(alpha).toBeGreaterThanOrEqual(0.6);
      expect(alpha).toBeLessThan(1);
      for (const text of ['--ink', '--ink-2', '--ink-3', '--amber-text', '--red-text', '--green-text', '--cyan-text']) {
        const color = out[text] ?? vars[text];
        expect(readsOver(vars[from], alpha, color.toLowerCase()), `${text} on ${name}`).toBe(true);
      }
    }
  });

  it('derives from custom colors, not fixed values', () => {
    const vars = { ...DARK, '--chrome': '#202a10', '--panel': '#283214' };
    const out = glassVars(reader(vars));
    expect(hexOf(out['--glass-chrome'])).toBe('#202a10');
    expect(hexOf(out['--glass-panel'])).toBe('#283214');
  });

  it.each(['dark', 'light'] as const)('keeps every %s preset readable', (theme) => {
    for (const id of PRESET_IDS) {
      const c = presetColors(theme, id);
      const vars = { ...(theme === 'dark' ? DARK : LIGHT), '--chrome': c.chrome, '--panel': c.panel, '--raised': c.raised, '--ink': c.ink, '--ink-2': c['ink-2'] };
      const out = glassVars(reader(vars));
      for (const name of ['--glass-chrome', '--glass-panel']) {
        const bg = hexOf(out[name]);
        expect(readsOver(bg, alphaOf(out[name]), c['ink-2']), `${id} ${name}`).toBe(true);
      }
    }
  });

  it('keeps state colors nearer their own hue than the text color', () => {
    const out = glassVars(reader(DARK));
    for (const k of ['--amber-text', '--red-text', '--green-text', '--cyan-text']) {
      const c = out[k] ?? DARK[k];
      expect(colorDistance(c, DARK[k]), k).toBeLessThan(colorDistance(c, DARK['--ink']));
    }
  });

  it('leaves a surface opaque when its color is not hex', () => {
    const out = glassVars(reader({ ...DARK, '--ink': 'white' }));
    expect(alphaOf(out['--glass-panel'])).toBe(1);
  });

  it('sets only the variables it can clear', () => {
    expect(Object.keys(glassVars(reader(DARK))).every((k) => GLASS_VARS.includes(k))).toBe(true);
  });
});

describe('glass css', () => {
  it('keeps the solid look opaque', () => {
    // With a transparent window, Solid relies on this to hide the desktop.
    expect(appCss).toMatch(/^body \{ background: var\(--panel\);/m);
  });

  it('turns the body clear only once vibrancy is on', () => {
    expect(appCss).toContain(':root[data-style="glass"][data-see-through] body { background: transparent; }');
  });

  it('keeps wells and the terminal out of the glass rules', () => {
    const glass = appCss.slice(appCss.indexOf('/* Glass'));
    expect(glass).not.toMatch(/\.well|\.terminal|xterm/);
  });
});

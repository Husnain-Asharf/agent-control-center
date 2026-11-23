import { afterEach, describe, expect, it, vi } from 'vitest';
import tokens from '../styles/tokens.css?raw';
import {
  COLOR_KEYS,
  COLOR_VARS,
  MIN_TEXT_CONTRAST,
  PRESET_IDS,
  STATE_KEYS,
  applyColors,
  closeStatePairs,
  colorDistance,
  colorVars,
  colorsKey,
  contrast,
  defaultChoice,
  isHex,
  lowContrastText,
  matchingPreset,
  parseColors,
  presetColors,
  statePartners,
} from './colors';
import type { Theme } from './types';

const THEMES: Theme[] = ['dark', 'light'];

/** The value of a variable in one theme block of tokens.css. */
function tokenValue(theme: Theme, name: string): string {
  const start = theme === 'dark' ? tokens.indexOf(':root, :root[data-theme="dark"]') : tokens.indexOf(':root[data-theme="light"]');
  const block = tokens.slice(start, tokens.indexOf('}', start));
  const m = block.match(new RegExp(`${name}:\\s*([^;]+);`));
  if (!m) throw new Error(`${name} missing in ${theme}`);
  return m[1].trim().toLowerCase();
}

describe('presets', () => {
  it('Default is the colors in tokens.css', () => {
    for (const t of THEMES) {
      const d = presetColors(t, 'default');
      for (const k of COLOR_KEYS) expect(d[k], `${t} ${k}`).toBe(tokenValue(t, `--${k}`));
    }
  });

  it('an unknown preset gives the default', () => {
    expect(presetColors('dark', 'nope')).toEqual(presetColors('dark', 'default'));
  });

  it('returns a copy', () => {
    presetColors('dark', 'default').ink = '#000000';
    expect(presetColors('dark', 'default').ink).toBe('#e8ecef');
  });

  it('every preset has readable text and state colors that stand apart', () => {
    for (const t of THEMES) {
      for (const id of PRESET_IDS) {
        const c = presetColors(t, id);
        for (const k of COLOR_KEYS) expect(isHex(c[k])).toBe(true);
        expect(contrast(c.ink, c.panel), `${t} ${id} ink`).toBeGreaterThanOrEqual(MIN_TEXT_CONTRAST);
        expect(contrast(c['ink-2'], c.panel), `${t} ${id} ink-2`).toBeGreaterThanOrEqual(MIN_TEXT_CONTRAST);
        expect(contrast(c['ink-2'], c.raised), `${t} ${id} ink-2 on cards`).toBeGreaterThanOrEqual(MIN_TEXT_CONTRAST);
        expect(closeStatePairs(c), `${t} ${id}`).toEqual([]);
      }
    }
  });

  it('matchingPreset names the saved preset until a color changes', () => {
    const warm = { preset: 'warm' as const, colors: presetColors('dark', 'warm') };
    expect(matchingPreset('dark', warm)).toBe('warm');
    expect(matchingPreset('dark', { ...warm, colors: { ...warm.colors, cyan: '#0000ff' } })).toBeNull();
    expect(matchingPreset('light', defaultChoice('light'))).toBe('default');
  });
});

describe('parseColors', () => {
  it('gives the default for nothing or broken data', () => {
    for (const saved of [null, '', '{', '42', 'null', '"x"']) {
      expect(parseColors('dark', saved)).toEqual(defaultChoice('dark'));
    }
  });

  it('reads a saved preset and colors', () => {
    const colors = { ...presetColors('light', 'blue'), amber: '#AA5500' };
    const got = parseColors('light', JSON.stringify({ preset: 'blue', colors }));
    expect(got.preset).toBe('blue');
    expect(got.colors.amber).toBe('#aa5500');
    expect(got.colors.panel).toBe(presetColors('light', 'blue').panel);
  });

  it('falls back per color to the preset value', () => {
    const got = parseColors('dark', JSON.stringify({ preset: 'warm', colors: { ink: 'red', red: '#12345', green: 7, cyan: '#123456' } }));
    const warm = presetColors('dark', 'warm');
    expect(got.colors.ink).toBe(warm.ink);
    expect(got.colors.red).toBe(warm.red);
    expect(got.colors.green).toBe(warm.green);
    expect(got.colors.cyan).toBe('#123456');
  });

  it('an unknown preset becomes the default', () => {
    expect(parseColors('dark', JSON.stringify({ preset: 'neon' })).preset).toBe('default');
  });

  it('keeps one key per theme', () => {
    expect(colorsKey('dark')).not.toBe(colorsKey('light'));
  });
});

describe('color math', () => {
  it('contrast matches known values', () => {
    expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 5);
    expect(contrast('#777777', '#777777')).toBeCloseTo(1, 5);
    expect(contrast('#767676', '#ffffff')).toBeCloseTo(4.54, 1);
  });

  it('distance is zero for the same color and large for opposites', () => {
    expect(colorDistance('#52c28b', '#52c28b')).toBe(0);
    expect(colorDistance('#000000', '#ffffff')).toBeCloseTo(100, 0);
  });
});

describe('closeStatePairs', () => {
  it('flags state colors that look alike', () => {
    const c = { ...presetColors('dark', 'default'), red: '#f0a040' };
    expect(closeStatePairs(c)).toEqual([['amber', 'red']]);
  });

  it('flags every close pair', () => {
    const c = { ...presetColors('dark', 'default'), red: '#63b4e6', green: '#63b4e6', amber: '#63b4e6' };
    expect(closeStatePairs(c)).toHaveLength(6);
  });
});

describe('lowContrastText', () => {
  it('lists text that is hard to read on the panels', () => {
    const c = { ...presetColors('dark', 'default'), 'ink-2': '#303840' };
    expect(lowContrastText(c)).toEqual(['ink-2']);
    expect(lowContrastText(presetColors('dark', 'default'))).toEqual([]);
  });
});

describe('statePartners', () => {
  it('keeps a readable color as its own text color', () => {
    const p = statePartners('#F0A43A', presetColors('dark', 'default'));
    expect(p.text).toBe('#f0a43a');
    expect(p.soft).toBe('rgba(240,164,58,0.13)');
  });

  it('moves a hard to read color toward the text color', () => {
    for (const t of THEMES) {
      const base = presetColors(t, 'default');
      const color = t === 'dark' ? '#552222' : '#ffcc66';
      const p = statePartners(color, base);
      expect(p.text).not.toBe(color);
      expect(contrast(p.text, base.panel)).toBeGreaterThanOrEqual(MIN_TEXT_CONTRAST);
    }
  });
});

describe('colorVars', () => {
  it('sets nothing for the default', () => {
    for (const t of THEMES) expect(colorVars(t, defaultChoice(t))).toEqual({});
  });

  it('sets a changed state color with its partners', () => {
    const choice = defaultChoice('dark');
    choice.colors.green = '#40d080';
    const vars = colorVars('dark', choice);
    expect(vars['--green']).toBe('#40d080');
    expect(vars['--green-text']).toBe('#40d080');
    expect(vars['--green-soft']).toBe('rgba(64,208,128,0.13)');
    expect(vars['--add-bg']).toBeDefined();
    expect(vars['--amber']).toBeUndefined();
    expect(vars['--amber-text']).toBeUndefined();
  });

  it('a preset brings its extra variables', () => {
    const vars = colorVars('dark', { preset: 'warm', colors: presetColors('dark', 'warm') });
    expect(vars['--panel']).toBe('#1a1814');
    expect(vars['--hover']).toBeDefined();
    expect(vars['--red']).toBeUndefined();
  });

  it('only sets variables COLOR_VARS clears', () => {
    for (const t of THEMES) {
      for (const id of PRESET_IDS) {
        const colors = presetColors(t, id);
        for (const k of STATE_KEYS) colors[k] = '#808080';
        for (const name of Object.keys(colorVars(t, { preset: id, colors }))) expect(COLOR_VARS).toContain(name);
      }
    }
  });
});

describe('applyColors', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('sets the variables on <html> and clears them for the default', () => {
    const props = new Map<string, string>();
    const style = {
      setProperty: (k: string, v: string) => props.set(k, v),
      removeProperty: (k: string) => props.delete(k),
    };
    vi.stubGlobal('document', { documentElement: { style } });
    applyColors('dark', { preset: 'blue', colors: presetColors('dark', 'blue') });
    expect(props.get('--panel')).toBe('#121925');
    expect(props.has('--hover')).toBe(true);
    applyColors('dark', defaultChoice('dark'));
    expect(props.size).toBe(0);
  });

  it('does nothing without a document', () => {
    expect(() => applyColors('dark', defaultChoice('dark'))).not.toThrow();
  });
});

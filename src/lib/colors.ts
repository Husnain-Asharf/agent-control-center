import type { Theme } from './types';

/** The colors Settings lets you pick. Each maps to a CSS variable in tokens.css. */
export const COLOR_KEYS = ['chrome', 'panel', 'raised', 'ink', 'ink-2', 'amber', 'red', 'green', 'cyan'] as const;
export type ColorKey = (typeof COLOR_KEYS)[number];
export type ColorSet = Record<ColorKey, string>;

/** The state colors. The app tells sessions apart by these, so they must not look alike. */
export const STATE_KEYS = ['amber', 'red', 'green', 'cyan'] as const;
export type StateKey = (typeof STATE_KEYS)[number];

export const COLOR_LABEL: Record<ColorKey, string> = {
  chrome: 'Window',
  panel: 'Panels',
  raised: 'Cards',
  ink: 'Main text',
  'ink-2': 'Muted text',
  amber: 'Needs you',
  red: 'Failed',
  green: 'Done',
  cyan: 'Working',
};

export const PRESET_IDS = ['default', 'contrast', 'warm', 'blue'] as const;
export type PresetId = (typeof PRESET_IDS)[number];

export const PRESET_LABEL: Record<PresetId, string> = {
  default: 'Default',
  contrast: 'High contrast',
  warm: 'Warm',
  blue: 'Blue',
};

interface Preset {
  colors: ColorSet;
  /** Other variables the preset changes so hovers, wells and lines match its greys. */
  extra: Record<string, string>;
}

const DARK_DEFAULT: ColorSet = {
  chrome: '#101417',
  panel: '#13181c',
  raised: '#1a2025',
  ink: '#e8ecef',
  'ink-2': '#a3aeb6',
  amber: '#f0a43a',
  red: '#ee6a5f',
  green: '#52c28b',
  cyan: '#63b4e6',
};

const LIGHT_DEFAULT: ColorSet = {
  chrome: '#ebeef0',
  panel: '#f4f6f7',
  raised: '#ffffff',
  ink: '#141c22',
  'ink-2': '#47535d',
  amber: '#e0921f',
  red: '#d9483b',
  green: '#2fa36b',
  cyan: '#2e8bc9',
};

const PRESETS: Record<Theme, Record<PresetId, Preset>> = {
  dark: {
    default: { colors: DARK_DEFAULT, extra: {} },
    contrast: {
      colors: {
        chrome: '#07090b',
        panel: '#0b0e11',
        raised: '#14191d',
        ink: '#ffffff',
        'ink-2': '#c4cdd3',
        amber: '#ffb547',
        red: '#ff7a6e',
        green: '#5fd69a',
        cyan: '#74c4f2',
      },
      extra: {
        '--hover': '#1c2328',
        '--well': '#000000',
        '--well-hover': '#05070a',
        '--ink-3': '#9aa6af',
        '--term-fg': '#e3e9ed',
        '--term-dim': '#93a0a9',
        '--line': 'rgba(255,255,255,.12)',
        '--line-strong': 'rgba(255,255,255,.2)',
      },
    },
    warm: {
      colors: { ...DARK_DEFAULT, chrome: '#15130f', panel: '#1a1814', raised: '#22201b', ink: '#eeeae2', 'ink-2': '#b0a99d' },
      extra: {
        '--hover': '#2a2721',
        '--well': '#0e0d0a',
        '--well-hover': '#13120e',
        '--ink-3': '#8c857a',
        '--term-fg': '#d3cdc2',
        '--term-dim': '#8a8377',
      },
    },
    blue: {
      colors: { ...DARK_DEFAULT, chrome: '#0e131b', panel: '#121925', raised: '#19222f', ink: '#e6ecf3', 'ink-2': '#a0adbd' },
      extra: {
        '--hover': '#1f2a39',
        '--well': '#090d14',
        '--well-hover': '#0d121a',
        '--ink-3': '#7d8a9b',
        '--term-fg': '#c5d0dc',
        '--term-dim': '#7b889a',
      },
    },
  },
  light: {
    default: { colors: LIGHT_DEFAULT, extra: {} },
    contrast: {
      colors: { ...LIGHT_DEFAULT, chrome: '#e2e6e9', panel: '#ffffff', raised: '#ffffff', ink: '#000000', 'ink-2': '#333d45' },
      extra: {
        '--hover': '#e4e8eb',
        '--ink-3': '#46515a',
        '--term-fg': '#111a20',
        '--term-dim': '#46515a',
        '--line': 'rgba(16,24,32,.16)',
        '--line-strong': 'rgba(16,24,32,.28)',
      },
    },
    warm: {
      colors: { ...LIGHT_DEFAULT, chrome: '#efece6', panel: '#f7f5f0', raised: '#fffefb', ink: '#1d1a15', 'ink-2': '#514b41' },
      extra: {
        '--hover': '#ece8e0',
        '--well': '#fffefb',
        '--well-hover': '#f8f6f1',
        '--ink-3': '#645d52',
        '--term-fg': '#2e2a23',
        '--term-dim': '#6a6357',
      },
    },
    blue: {
      colors: { ...LIGHT_DEFAULT, chrome: '#e8edf3', panel: '#f2f5f9', raised: '#ffffff', ink: '#121b26', 'ink-2': '#435163' },
      extra: {
        '--hover': '#e5ebf2',
        '--well-hover': '#f4f7fa',
        '--ink-3': '#566577',
        '--term-fg': '#22303f',
        '--term-dim': '#5c6a7b',
      },
    },
  },
};

/** The colors of a preset. Unknown ids give the default. */
export function presetColors(theme: Theme, id: string): ColorSet {
  const p = PRESETS[theme][id as PresetId] ?? PRESETS[theme].default;
  return { ...p.colors };
}

/** What is saved for one theme: the preset it started from and the colors now in use. */
export interface ColorChoice {
  preset: PresetId;
  colors: ColorSet;
}

export const defaultChoice = (theme: Theme): ColorChoice => ({ preset: 'default', colors: presetColors(theme, 'default') });

/** Storage key for a theme's colors. Dark and light keep their own. */
export const colorsKey = (theme: Theme): string => `acc-colors-${theme}`;

const HEX = /^#[0-9a-f]{6}$/i;
export const isHex = (v: unknown): v is string => typeof v === 'string' && HEX.test(v);

/**
 * The saved colors of a theme. Missing or broken data gives the default, and a color that
 * is not a six digit hex falls back to its preset value.
 */
export function parseColors(theme: Theme, saved: string | null): ColorChoice {
  if (!saved) return defaultChoice(theme);
  let data: unknown;
  try {
    data = JSON.parse(saved);
  } catch {
    return defaultChoice(theme);
  }
  if (!data || typeof data !== 'object') return defaultChoice(theme);
  const raw = data as { preset?: unknown; colors?: unknown };
  const preset = PRESET_IDS.find((p) => p === raw.preset) ?? 'default';
  const base = presetColors(theme, preset);
  const given = raw.colors && typeof raw.colors === 'object' ? (raw.colors as Record<string, unknown>) : {};
  const colors = { ...base };
  for (const k of COLOR_KEYS) {
    const v = given[k];
    if (isHex(v)) colors[k] = v.toLowerCase();
  }
  return { preset, colors };
}

/** The saved preset while its colors are all unchanged, or null once one was picked by hand. */
export function matchingPreset(theme: Theme, choice: ColorChoice): PresetId | null {
  const base = presetColors(theme, choice.preset);
  return COLOR_KEYS.every((k) => base[k] === choice.colors[k].toLowerCase()) ? choice.preset : null;
}

// Color math

type Rgb = [number, number, number];

function toRgb(hex: string): Rgb {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

const toHex = (rgb: Rgb): string =>
  `#${rgb.map((c) => Math.round(Math.min(255, Math.max(0, c))).toString(16).padStart(2, '0')).join('')}`;

const linear = (c: number): number => {
  const s = c / 255;
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
};

function luminance(hex: string): number {
  const [r, g, b] = toRgb(hex).map(linear);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio of two colors, from 1 to 21. */
export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

function toLab(hex: string): Rgb {
  const [r, g, b] = toRgb(hex).map(linear);
  const x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
  const y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  const z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
  const f = (t: number) => (t > 0.008856 ? Math.cbrt(t) : 7.787 * t + 16 / 116);
  const [fx, fy, fz] = [f(x), f(y), f(z)];
  return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)];
}

/** How far apart two colors look (CIE76 delta E). Below about 2 most people see no difference. */
export function colorDistance(a: string, b: string): number {
  const [l1, a1, b1] = toLab(a);
  const [l2, a2, b2] = toLab(b);
  return Math.hypot(l1 - l2, a1 - a2, b1 - b2);
}

/** State colors closer than this are easy to mix up at a glance. */
export const MIN_STATE_DISTANCE = 25;

/** Pairs of state colors that are too close to tell apart. */
export function closeStatePairs(colors: ColorSet): [StateKey, StateKey][] {
  const out: [StateKey, StateKey][] = [];
  STATE_KEYS.forEach((a, i) => {
    for (const b of STATE_KEYS.slice(i + 1)) {
      if (colorDistance(colors[a], colors[b]) < MIN_STATE_DISTANCE) out.push([a, b]);
    }
  });
  return out;
}

/** WCAG AA for normal text. */
export const MIN_TEXT_CONTRAST = 4.5;

/** Text colors that are hard to read on the panels. */
export const lowContrastText = (colors: ColorSet): ColorKey[] =>
  (['ink', 'ink-2'] as const).filter((k) => contrast(colors[k], colors.panel) < MIN_TEXT_CONTRAST);

const mix = (a: string, b: string, t: number): string => {
  const [x, y] = [toRgb(a), toRgb(b)];
  return toHex([0, 1, 2].map((i) => x[i] + (y[i] - x[i]) * t) as Rgb);
};

const rgba = (hex: string, alpha: number): string => `rgba(${toRgb(hex).join(',')},${alpha})`;

/**
 * The partners of a state color. `-text` is the color itself, moved toward the text color
 * until it reads on the panels (4.5:1). `-soft` is the color at low alpha, for tinted fills.
 */
export function statePartners(color: string, colors: Pick<ColorSet, 'panel' | 'ink'>): { text: string; soft: string } {
  let text = color.toLowerCase();
  for (let t = 0.1; contrast(text, colors.panel) < MIN_TEXT_CONTRAST && t <= 1; t += 0.1) {
    text = mix(color, colors.ink, t);
  }
  return { text, soft: rgba(color, 0.13) };
}

/** Every variable colors may set, so the old ones can be cleared on a change. */
export const COLOR_VARS: string[] = (() => {
  const names = new Set<string>(COLOR_KEYS.map((k) => `--${k}`));
  for (const k of STATE_KEYS) names.add(`--${k}-text`).add(`--${k}-soft`);
  names.add('--add-bg').add('--del-bg');
  for (const t of ['dark', 'light'] as const) {
    for (const id of PRESET_IDS) for (const v of Object.keys(PRESETS[t][id].extra)) names.add(v);
  }
  return [...names];
})();

/**
 * The CSS variables to set on <html> for a theme's colors. Only what differs from tokens.css
 * is listed, so the default gives an empty map.
 */
export function colorVars(theme: Theme, choice: ColorChoice): Record<string, string> {
  const preset = PRESETS[theme][choice.preset] ?? PRESETS[theme].default;
  const base = PRESETS[theme].default.colors;
  const vars: Record<string, string> = { ...preset.extra };
  const { colors } = choice;
  for (const k of COLOR_KEYS) {
    if (colors[k].toLowerCase() !== base[k]) vars[`--${k}`] = colors[k];
  }
  for (const k of STATE_KEYS) {
    if (colors[k].toLowerCase() === base[k]) continue;
    const { text, soft } = statePartners(colors[k], colors);
    vars[`--${k}-text`] = text;
    vars[`--${k}-soft`] = soft;
    if (k === 'green') vars['--add-bg'] = rgba(colors[k], 0.11);
    if (k === 'red') vars['--del-bg'] = rgba(colors[k], 0.1);
  }
  return vars;
}

/** Sets a theme's colors on <html>, clearing what an earlier choice set. */
export function applyColors(theme: Theme, choice: ColorChoice): void {
  const style = typeof document === 'undefined' ? undefined : document.documentElement?.style;
  if (!style) return;
  for (const name of COLOR_VARS) style.removeProperty(name);
  for (const [name, value] of Object.entries(colorVars(theme, choice))) style.setProperty(name, value);
}

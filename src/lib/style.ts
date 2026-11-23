import { isTauri } from '@tauri-apps/api/core';
import { Effect, EffectState, getCurrentWindow } from '@tauri-apps/api/window';
import { contrast, isHex, luminance, MIN_TEXT_CONTRAST, mix, rgba } from './colors';
import { isMac, type Platform } from './platform';
import type { Theme } from './types';

/** How panels are drawn. Solid is the default. Glass makes the frame translucent. */
export const STYLES = ['solid', 'glass'] as const;
export type Style = (typeof STYLES)[number];
export const DEFAULT_STYLE: Style = 'solid';

export const STYLE_LABEL: Record<Style, string> = { solid: 'Solid', glass: 'Glass' };

export const parseStyle = (saved: string | null): Style => STYLES.find((s) => s === saved) ?? DEFAULT_STYLE;

/** What Glass shows through on each platform. Only macOS gets real window vibrancy. */
export const showsDesktop = (on: Platform): boolean => on === 'mac';

/** The Settings note under Style. */
export function styleNote(on: Platform): string {
  if (showsDesktop(on)) return 'Glass lets the desktop show through the window, softly blurred.';
  const name = on === 'windows' ? 'Windows' : 'Linux';
  return `Glass draws translucent panels over the app's own background. The desktop does not show through on ${name}.`;
}

// Glass colors

/**
 * Anything can sit behind a glass panel: the desktop, or app content scrolling under it.
 * Black and white are the extremes, so text that reads over both reads over anything.
 */
const BACKDROPS = ['#000000', '#ffffff'] as const;

/**
 * Whether `text` keeps AA contrast on `bg` drawn at `alpha` over any backdrop. It must also
 * stay on the same side of the text in lightness, or some backdrop in between would match it.
 */
export function readsOver(bg: string, alpha: number, text: string): boolean {
  const side = Math.sign(luminance(bg) - luminance(text));
  return BACKDROPS.every((back) => {
    const seen = mix(back, bg, alpha);
    return Math.sign(luminance(seen) - luminance(text)) === side && contrast(seen, text) >= MIN_TEXT_CONTRAST;
  });
}

/** The lowest alpha, from `floor` up, at which every text color reads on `bg`. 1 when none does. */
export function glassAlpha(bg: string, texts: string[], floor: number): number {
  for (let a = Math.round(floor * 100); a < 100; a++) {
    if (texts.every((t) => readsOver(bg, a / 100, t))) return a / 100;
  }
  return 1;
}

/** A text color moved toward `ink` until it reads on every glass surface. */
export function liftText(color: string, ink: string, surfaces: { bg: string; alpha: number }[]): string {
  const reads = (c: string) => surfaces.every((s) => readsOver(s.bg, s.alpha, c));
  let out = color.toLowerCase();
  for (let t = 0.1; !reads(out) && t <= 1.001; t += 0.1) out = mix(color, ink, Math.min(1, t));
  return out;
}

/** The surfaces that turn to glass, and how see-through each one is at most. */
const SURFACES = [
  { name: '--glass-chrome', from: '--chrome', floor: 0.6 },
  { name: '--glass-panel', from: '--panel', floor: 0.78 },
  { name: '--glass-raised', from: '--raised', floor: 0.85 },
] as const;

/** Text colors that are kept AA by their alpha. */
const MAIN_TEXT = ['--ink', '--ink-2'] as const;

/** Fainter text colors. Glass moves them toward --ink where they would not read. */
const WEAK_TEXT = ['--ink-3', '--amber-text', '--red-text', '--green-text', '--cyan-text'] as const;

/** Every variable glassVars may set, so the old ones can be cleared. */
export const GLASS_VARS: string[] = [...SURFACES.map((s) => s.name), ...WEAK_TEXT];

/**
 * The CSS variables of the glass look, derived from the colors now in use. `read` gives a
 * variable's current value. Each surface gets the lowest alpha at which the main text still
 * reads over any backdrop, and fainter text is lifted only as far as it needs. A color that
 * is not hex leaves its surface opaque and its text as it is.
 */
export function glassVars(read: (name: string) => string): Record<string, string> {
  const val = (name: string) => read(name).trim().toLowerCase();
  const texts = MAIN_TEXT.map(val);
  const textsOk = texts.every(isHex);
  const vars: Record<string, string> = {};
  const surfaces: { bg: string; alpha: number }[] = [];
  for (const s of SURFACES) {
    const bg = val(s.from);
    if (!isHex(bg)) continue;
    const alpha = textsOk ? glassAlpha(bg, texts, s.floor) : 1;
    vars[s.name] = rgba(bg, alpha);
    surfaces.push({ bg, alpha });
  }
  const ink = val('--ink');
  if (isHex(ink)) {
    for (const name of WEAK_TEXT) {
      const color = val(name);
      if (!isHex(color)) continue;
      const lifted = liftText(color, ink, surfaces);
      if (lifted !== color) vars[name] = lifted;
    }
  }
  return vars;
}

// Applying

let applied = 0;
/** Whether this window has vibrancy on, so Solid knows to clear it. */
let windowGlass = false;

/**
 * Applies a style. It marks <html> with data-style, puts the glass colors on <body> (so the
 * colors on <html> stay the source they derive from), and on macOS turns window vibrancy on or
 * off. Call it again after the theme or colors change. The vibrancy is async: data-see-through
 * goes on <html> once it is in place, and the CSS keeps an opaque background until then.
 */
export function applyStyle(style: Style, theme: Theme): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  const body = document.body;
  root.dataset.style = style;
  if (body) for (const name of GLASS_VARS) body.style.removeProperty(name);
  if (style === 'solid') {
    delete root.dataset.seeThrough;
    if (windowGlass) setWindowGlass(false, theme);
    return;
  }
  setGlassColors(true);
  if (isMac && isTauri()) setWindowGlass(true, theme);
}

/**
 * Puts the glass colors on <body>. In dev the stylesheets load after the store, so at startup
 * there may be no colors to read yet; then it tries once more on the next frame.
 */
function setGlassColors(retry: boolean): void {
  const root = document.documentElement;
  const body = document.body;
  if (!body || root.dataset.style !== 'glass') return;
  const css = getComputedStyle(root);
  const vars = glassVars((n) => css.getPropertyValue(n));
  if (Object.keys(vars).length === 0 && retry && typeof requestAnimationFrame !== 'undefined') {
    requestAnimationFrame(() => setGlassColors(false));
    return;
  }
  for (const [name, value] of Object.entries(vars)) body.style.setProperty(name, value);
}

function setWindowGlass(on: boolean, theme: Theme): void {
  const call = ++applied;
  const root = document.documentElement;
  windowGlass = on;
  try {
    const win = getCurrentWindow();
    const done = on
      ? Promise.all([
          // The material follows the window's appearance, so match it to the app theme.
          win.setTheme(theme),
          win.setEffects({ effects: [Effect.Sidebar], state: EffectState.FollowsWindowActiveState }),
        ])
      : Promise.all([win.clearEffects(), win.setTheme(null)]);
    done
      .then(() => {
        if (call === applied && on) root.dataset.seeThrough = '';
      })
      .catch(() => {
        // No vibrancy: the CSS look stays.
        if (call === applied) delete root.dataset.seeThrough;
      });
  } catch {
    // no window
  }
}

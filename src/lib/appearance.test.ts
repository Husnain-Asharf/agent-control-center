import { afterEach, describe, expect, it, vi } from 'vitest';
import appCss from '../styles/app.css?raw';
import tokens from '../styles/tokens.css?raw';
import fontsCss from '../styles/fonts.css?raw';
import { DEFAULT_TERM_FONT_SIZE, DEFAULT_TEXT_SCALE, TEXT_SCALES, applyTextScale, fittedScale, parseBolder, parseTermFontSize, parseTextScale, scaleLabel, setBolderAttr, stepTextScale, termFontWeight, type TextScale } from './appearance';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('saved text settings', () => {
  it.each(['0.9', '1', '1.1', '1.25', '1.4'])('reads text size %s', (saved) => {
    expect(parseTextScale(saved)).toBe(Number(saved));
  });
  it.each([null, '', '1.5', '110%', 'big'])('falls back to 100%% for %j', (saved) => {
    expect(parseTextScale(saved)).toBe(DEFAULT_TEXT_SCALE);
  });
  it.each(['11', '12.5', '14', '16'])('reads terminal size %s', (saved) => {
    expect(parseTermFontSize(saved)).toBe(Number(saved));
  });
  it.each([null, '', '13', '12.5px'])('falls back to 12.5 for %j', (saved) => {
    expect(parseTermFontSize(saved)).toBe(DEFAULT_TERM_FONT_SIZE);
  });
  it('turns bolder text on only for a saved on', () => {
    expect(parseBolder('on')).toBe(true);
    for (const saved of [null, '', 'off', 'true', '1']) expect(parseBolder(saved)).toBe(false);
  });
});

describe('stepTextScale', () => {
  it('walks the presets in order', () => {
    const up: TextScale[] = [TEXT_SCALES[0]];
    while (up.length < TEXT_SCALES.length) up.push(stepTextScale(up[up.length - 1], 1));
    expect(up).toEqual([...TEXT_SCALES]);
    expect(stepTextScale(1.25, -1)).toBe(1.1);
    expect(stepTextScale(1, -1)).toBe(0.9);
  });
  it('stays put at either end', () => {
    expect(stepTextScale(1.4, 1)).toBe(1.4);
    expect(stepTextScale(0.9, -1)).toBe(0.9);
  });
});

describe('labels and weights', () => {
  it('shows text sizes as percentages', () => {
    expect(TEXT_SCALES.map(scaleLabel)).toEqual(['90%', '100%', '110%', '125%', '140%']);
  });
  it('makes the terminal one step heavier with bolder text', () => {
    expect(termFontWeight(false)).toBe(400);
    expect(termFontWeight(true)).toBe(500);
  });
});

describe('applyTextScale', () => {
  it('does nothing outside the app', () => {
    expect(() => applyTextScale(1.25)).not.toThrow();
  });
  it('zooms the webview in the app', () => {
    const invoke = vi.fn(() => Promise.resolve());
    const metadata = { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } };
    vi.stubGlobal('isTauri', true);
    vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke, metadata } });
    applyTextScale(1.25);
    expect(invoke).toHaveBeenCalledWith('plugin:webview|set_webview_zoom', { label: 'main', value: 1.25 }, undefined);
  });
  it('caps the zoom so the page stays at least 1024 pixels wide', () => {
    expect(fittedScale(1.4, 1440)).toBe(1.4);
    expect(fittedScale(1.4, 1200)).toBeCloseTo(1200 / 1024);
    expect(fittedScale(1.25, 1024)).toBe(1);
    expect(fittedScale(0.9, 1024)).toBe(0.9);
    expect(fittedScale(1.1, Number.POSITIVE_INFINITY)).toBe(1.1);
  });
  it('survives a refused zoom', async () => {
    vi.stubGlobal('isTauri', true);
    vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke: () => Promise.reject(new Error('not allowed')) } });
    expect(() => applyTextScale(1.1)).not.toThrow();
  });
});

describe('setBolderAttr', () => {
  it('sets and clears data-bolder on <html>', () => {
    const root = { dataset: {} as Record<string, string> };
    vi.stubGlobal('document', { documentElement: root });
    setBolderAttr(true);
    expect('bolder' in root.dataset).toBe(true);
    setBolderAttr(false);
    expect('bolder' in root.dataset).toBe(false);
  });
  it('does nothing without a document', () => {
    expect(() => setBolderAttr(true)).not.toThrow();
  });
});

/** The weights fonts.css bundles for a family, from its @font-face rules. */
function loadedWeights(family: string): number[] {
  const faces = [...fontsCss.matchAll(/@font-face\s*\{([^}]*)\}/g)].map((m) => m[1]);
  return faces
    .filter((f) => new RegExp(`font-family:\\s*"${family}"`).test(f))
    .map((f) => Number(/font-weight:\s*(\d+)/.exec(f)?.[1]))
    .sort((a, b) => a - b);
}

/** The weight variables one rule of tokens.css sets. */
function weightVars(selector: string): Record<string, number> {
  const start = tokens.indexOf(`${selector} {`);
  if (start === -1) throw new Error(`no ${selector} rule in tokens.css`);
  const body = tokens.slice(start, tokens.indexOf('}', start));
  return Object.fromEntries([...body.matchAll(/--(w-[a-z-]+):\s*(\d+)/g)].map((m) => [m[1], Number(m[2])]));
}

describe('bolder text weights', () => {
  const ui = loadedWeights('Geist');
  const code = loadedWeights('Geist Mono');
  const base = weightVars(':root');
  const bolder = { ...base, ...weightVars(':root[data-bolder]') };
  const steps = ['w-regular', 'w-medium', 'w-semibold'];

  it('finds the bundled weights in fonts.css', () => {
    expect(ui.length).toBeGreaterThan(0);
    expect(code.length).toBeGreaterThan(0);
  });
  it('starts at the weights app.css had before', () => {
    expect(base).toMatchObject({ 'w-regular': 400, 'w-medium': 500, 'w-semibold': 600 });
  });
  it('raises each weight one step, up to the heaviest Geist loaded', () => {
    const top = Math.max(...ui);
    for (const k of steps) expect(bolder[k]).toBe(Math.min(base[k] + 100, top));
  });
  it('uses only loaded weights, so no bold is faked', () => {
    for (const set of [base, bolder]) {
      for (const k of steps) expect(ui).toContain(set[k]);
      // Code text inherits regular and medium, and has its own semibold.
      for (const k of ['w-regular', 'w-medium', 'w-code-semibold']) expect(code).toContain(set[k]);
    }
  });
  it('leaves no fixed font weight in app.css', () => {
    expect(appCss).not.toMatch(/font-weight:\s*\d/);
  });
});

import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';

/** Text size presets for the whole window, as page zoom. 1 is 100%. */
export const TEXT_SCALES = [0.9, 1, 1.1, 1.25, 1.4] as const;
export type TextScale = (typeof TEXT_SCALES)[number];
export const DEFAULT_TEXT_SCALE: TextScale = 1;

/** Font size presets for session terminals, in CSS pixels. */
export const TERM_FONT_SIZES = [11, 12.5, 14, 16] as const;
export type TermFontSize = (typeof TERM_FONT_SIZES)[number];
export const DEFAULT_TERM_FONT_SIZE: TermFontSize = 12.5;

/** The preset a saved string names, or `fallback` when it is missing, invalid or no longer a preset. */
function preset<T extends number>(list: readonly T[], saved: string | null, fallback: T): T {
  const n = Number(saved);
  return list.find((v) => v === n) ?? fallback;
}

export const parseTextScale = (saved: string | null): TextScale => preset(TEXT_SCALES, saved, DEFAULT_TEXT_SCALE);

export const parseTermFontSize = (saved: string | null): TermFontSize =>
  preset(TERM_FONT_SIZES, saved, DEFAULT_TERM_FONT_SIZE);

/** Bolder text is off unless it was saved as on. */
export const parseBolder = (saved: string | null): boolean => saved === 'on';

/** The next text size up (1) or down (-1). It stays put at either end. */
export function stepTextScale(current: TextScale, dir: 1 | -1): TextScale {
  const at = TEXT_SCALES.indexOf(current);
  const from = at === -1 ? TEXT_SCALES.indexOf(DEFAULT_TEXT_SCALE) : at;
  return TEXT_SCALES[Math.min(TEXT_SCALES.length - 1, Math.max(0, from + dir))];
}

/** A text size as the UI shows it, such as 125%. */
export const scaleLabel = (scale: number): string => `${Math.round(scale * 100)}%`;

/**
 * The terminal's normal font weight. index.html loads Geist Mono at 400, 500 and 600, and
 * bold terminal text already uses 600, the heaviest, so only the normal weight goes up.
 */
export const termFontWeight = (bolder: boolean): number => (bolder ? 500 : 400);

/**
 * Zooms the app's webview. Page zoom keeps mouse selection in the terminal working, which
 * CSS zoom does not. Does nothing outside the app, such as in tests or a plain browser.
 */
export function applyTextScale(scale: number): void {
  if (!isTauri()) return;
  try {
    getCurrentWebview()
      .setZoom(scale)
      .catch(() => {});
  } catch {
    // no webview to zoom
  }
}

/** Marks <html> with data-bolder while bolder text is on. tokens.css raises the weights from it. */
export function setBolderAttr(on: boolean): void {
  if (typeof document === 'undefined') return;
  const { dataset } = document.documentElement;
  if (on) dataset.bolder = '';
  else delete dataset.bolder;
}

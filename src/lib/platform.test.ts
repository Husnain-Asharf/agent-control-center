import { describe, expect, it } from 'vitest';
import { isMacUserAgent, isWindowsUserAgent, keyHints, platformOf } from './platform';
import { matchShortcut } from './keys';

const MAC = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)';
const LINUX = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15';
const WINDOWS =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0';

describe('isMacUserAgent', () => {
  it('is true for the macOS webview', () => {
    expect(isMacUserAgent(MAC)).toBe(true);
  });
  it('is false for the Linux and Windows webviews', () => {
    expect(isMacUserAgent(LINUX)).toBe(false);
    expect(isMacUserAgent(WINDOWS)).toBe(false);
    expect(isMacUserAgent('')).toBe(false);
  });
});

describe('isWindowsUserAgent', () => {
  it('is true for the Windows webview', () => {
    expect(isWindowsUserAgent(WINDOWS)).toBe(true);
  });
  it('is false for the macOS and Linux webviews', () => {
    expect(isWindowsUserAgent(MAC)).toBe(false);
    expect(isWindowsUserAgent(LINUX)).toBe(false);
    expect(isWindowsUserAgent('')).toBe(false);
  });
});

describe('platformOf', () => {
  it('names the platform of each webview', () => {
    expect(platformOf(MAC)).toBe('mac');
    expect(platformOf(WINDOWS)).toBe('windows');
    expect(platformOf(LINUX)).toBe('linux');
  });
});

describe('keyHints', () => {
  it('shows the Cmd keys on macOS', () => {
    expect(keyHints('mac')).toEqual({
      palette: '⌘K',
      nextNeeds: '⌘J',
      textSize: { bigger: '⌘=', smaller: '⌘-', reset: '⌘0' },
    });
  });
  it('shows Ctrl+K on Windows and Linux, and no key for the jump', () => {
    const ctrl = { palette: 'Ctrl+K', nextNeeds: null, textSize: { bigger: 'Ctrl+=', smaller: 'Ctrl+-', reset: 'Ctrl+0' } };
    expect(keyHints('windows')).toEqual(ctrl);
    expect(keyHints('linux')).toEqual(ctrl);
  });
  it('names keys that work', () => {
    const press = (key: string, mods: { metaKey?: boolean; ctrlKey?: boolean }) => ({
      key,
      metaKey: false,
      ctrlKey: false,
      shiftKey: false,
      ...mods,
    });
    expect(matchShortcut(press('k', { metaKey: true }), false)).toEqual({ action: 'palette' });
    expect(matchShortcut(press('j', { metaKey: true }), false)).toEqual({ action: 'next-needs' });
    expect(matchShortcut(press('k', { ctrlKey: true }), false)).toEqual({ action: 'palette' });
    // Ctrl+J stays with the terminal.
    expect(matchShortcut(press('j', { ctrlKey: true }), false)).toBeNull();
    for (const mods of [{ metaKey: true }, { ctrlKey: true }]) {
      expect(matchShortcut(press('=', mods), false)).toEqual({ action: 'text-bigger' });
      expect(matchShortcut(press('-', mods), false)).toEqual({ action: 'text-smaller' });
      expect(matchShortcut(press('0', mods), false)).toEqual({ action: 'text-reset' });
    }
  });
});

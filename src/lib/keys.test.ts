import { describe, expect, it } from 'vitest';
import { isTypingTarget, matchShortcut, shortcutAllowed } from './keys';

const key = (k: string, mods: { meta?: boolean; ctrl?: boolean; shift?: boolean } = {}) => ({
  key: k,
  metaKey: !!mods.meta,
  ctrlKey: !!mods.ctrl,
  shiftKey: !!mods.shift,
});

describe('matchShortcut', () => {
  it('Cmd+K opens the palette', () => {
    expect(matchShortcut(key('k', { meta: true }), false)).toEqual({ action: 'palette' });
  });
  it('Ctrl+K opens the palette', () => {
    expect(matchShortcut(key('k', { ctrl: true }), false)).toEqual({ action: 'palette' });
  });
  it('Cmd+J jumps to the next thing that needs you', () => {
    expect(matchShortcut(key('j', { meta: true }), false)).toEqual({ action: 'next-needs' });
  });
  it('n starts a new session', () => {
    expect(matchShortcut(key('n'), false)).toEqual({ action: 'new-session' });
  });
  it('plain g only arms', () => {
    expect(matchShortcut(key('g'), false)).toEqual({ action: 'arm-g' });
  });
  it.each([
    ['o', 'go:overview'],
    ['s', 'go:sessions'],
    ['i', 'go:approvals'],
    ['p', 'go:projects'],
    ['a', 'go:activity'],
  ])('g then %s gives %s', (k, action) => {
    expect(matchShortcut(key(k), true)).toEqual({ action });
  });
  it('g then an unmapped key gives nothing', () => {
    expect(matchShortcut(key('x'), true)).toBeNull();
  });
  it('chord keys without the g prefix do nothing', () => {
    expect(matchShortcut(key('o'), false)).toBeNull();
    expect(matchShortcut(key('i'), false)).toBeNull();
  });
  it('plain keys with a modifier do nothing', () => {
    expect(matchShortcut(key('n', { meta: true }), false)).toBeNull();
    expect(matchShortcut(key('g', { ctrl: true }), false)).toBeNull();
    expect(matchShortcut(key('o', { meta: true }), true)).toBeNull();
  });
  it.each([
    ['=', 'text-bigger'],
    ['+', 'text-bigger'],
    ['-', 'text-smaller'],
    ['0', 'text-reset'],
  ])('Cmd or Ctrl with %s gives %s', (k, action) => {
    expect(matchShortcut(key(k, { meta: true }), false)).toEqual({ action });
    expect(matchShortcut(key(k, { ctrl: true }), false)).toEqual({ action });
  });
  it('text size keys need Cmd or Ctrl, and Ctrl with _ stays with the terminal', () => {
    expect(matchShortcut(key('='), false)).toBeNull();
    expect(matchShortcut(key('-'), false)).toBeNull();
    expect(matchShortcut(key('0'), false)).toBeNull();
    expect(matchShortcut(key('_', { ctrl: true, shift: true }), false)).toBeNull();
  });
});

type Stub = { tagName?: string; isContentEditable?: boolean; closest?: (s: string) => unknown };
const el = (s: Stub) => s as unknown as Element;

describe('isTypingTarget', () => {
  it('is false for null and plain elements', () => {
    expect(isTypingTarget(null)).toBe(false);
    expect(isTypingTarget(el({ tagName: 'DIV', closest: () => null }))).toBe(false);
  });
  it.each(['INPUT', 'TEXTAREA', 'SELECT'])('is true for %s', (tagName) => {
    expect(isTypingTarget(el({ tagName, closest: () => null }))).toBe(true);
  });
  it('is true for contentEditable', () => {
    expect(isTypingTarget(el({ tagName: 'DIV', isContentEditable: true, closest: () => null }))).toBe(true);
  });
  it('is true inside the terminal or a dialog', () => {
    const inside = (sel: string) => el({ tagName: 'DIV', closest: (s) => (s.split(', ').includes(sel) ? {} : null) });
    expect(isTypingTarget(inside('.xterm'))).toBe(true);
    expect(isTypingTarget(inside('[role="dialog"]'))).toBe(true);
  });
});

describe('isTypingTarget with a stub that has no closest', () => {
  it('is false for a plain element stub', () => {
    expect(isTypingTarget({ tagName: 'DIV' })).toBe(false);
  });
});

describe('shortcutAllowed', () => {
  const free = { typing: false, dialogOpen: false, paletteOpen: false };
  it('Cmd+K works from inputs and while the palette is open', () => {
    expect(shortcutAllowed('palette', { ...free, typing: true })).toBe(true);
    expect(shortcutAllowed('palette', { ...free, paletteOpen: true, typing: true })).toBe(true);
  });
  it('Cmd+K does nothing while a dialog is open', () => {
    expect(shortcutAllowed('palette', { ...free, dialogOpen: true })).toBe(false);
  });
  it('Cmd+J works from inputs but not over a dialog or the palette', () => {
    expect(shortcutAllowed('next-needs', { ...free, typing: true })).toBe(true);
    expect(shortcutAllowed('next-needs', { ...free, dialogOpen: true })).toBe(false);
    expect(shortcutAllowed('next-needs', { ...free, paletteOpen: true })).toBe(false);
  });
  it.each(['text-bigger', 'text-smaller', 'text-reset'])('%s works everywhere', (a) => {
    expect(shortcutAllowed(a, free)).toBe(true);
    expect(shortcutAllowed(a, { typing: true, dialogOpen: true, paletteOpen: true })).toBe(true);
  });
  it.each(['new-session', 'arm-g', 'go:sessions'])('plain %s needs free focus and no dialog', (a) => {
    expect(shortcutAllowed(a, free)).toBe(true);
    expect(shortcutAllowed(a, { ...free, typing: true })).toBe(false);
    expect(shortcutAllowed(a, { ...free, dialogOpen: true })).toBe(false);
    expect(shortcutAllowed(a, { ...free, paletteOpen: true })).toBe(false);
  });
});

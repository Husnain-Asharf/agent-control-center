export interface KeyInfo {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
}

/** What a key handler needs from a keydown, including the Approvals screen's extra fields. */
export type KeyTarget = { tagName?: string; isContentEditable?: boolean; closest?: (sel: string) => unknown };

export interface KeyLike extends Omit<KeyInfo, 'shiftKey'> {
  altKey: boolean;
  shiftKey?: boolean;
  repeat?: boolean;
  defaultPrevented?: boolean;
  target: KeyTarget | null;
}

export interface Shortcut {
  action: string;
}

const GO: Record<string, string> = {
  o: 'go:overview',
  s: 'go:sessions',
  i: 'go:approvals',
  p: 'go:projects',
  a: 'go:activity',
};

/**
 * Maps a key press to an action. `pendingG` is true after a plain `g`.
 * `g` alone returns `arm-g`; the caller arms the chord and clears it after the next key.
 * The caller skips typing targets (see isTypingTarget).
 */
export function matchShortcut(e: KeyInfo, pendingG: boolean): Shortcut | null {
  const key = e.key.toLowerCase();
  if (e.metaKey || e.ctrlKey) {
    if (key === 'k') return { action: 'palette' };
    if (key === 'j' && e.metaKey) return { action: 'next-needs' };
    // `+` is shift and `=` on a US keyboard. Ctrl with `_` stays with the terminal.
    if (key === '=' || key === '+') return { action: 'text-bigger' };
    if (key === '-') return { action: 'text-smaller' };
    if (key === '0') return { action: 'text-reset' };
    return null;
  }
  if (pendingG) return GO[key] ? { action: GO[key] } : null;
  if (key === 'g') return { action: 'arm-g' };
  if (key === 'n') return { action: 'new-session' };
  return null;
}

/** True when a key press belongs to the focused element, not to a shortcut. The only typing guard in the app. */
export function isTypingTarget(el: KeyTarget | null): boolean {
  if (!el) return false;
  const tag = (el.tagName ?? '').toUpperCase();
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
  if (el.isContentEditable) return true;
  return Boolean(el.closest?.('.xterm, [role="dialog"]'));
}

export interface ShortcutContext {
  /** Focus is in an input, the terminal or a dialog. */
  typing: boolean;
  /** A dialog other than the palette is open. */
  dialogOpen: boolean;
  paletteOpen: boolean;
}

/**
 * Whether the global handler may run an action now.
 * Cmd/Ctrl+K works from inputs and toggles the palette, but not over another dialog.
 * Cmd+J works from inputs but not over any dialog. Plain keys need a free focus and no dialog.
 * The text size keys work everywhere: inputs and the terminal do nothing with them.
 */
export function shortcutAllowed(action: string, ctx: ShortcutContext): boolean {
  if (action.startsWith('text-')) return true;
  if (action === 'palette') return !ctx.dialogOpen;
  if (ctx.dialogOpen || ctx.paletteOpen) return false;
  if (action === 'next-needs') return true;
  return !ctx.typing;
}

/**
 * Whether a webview's user agent is the macOS one. The app has no OS plugin; the
 * macOS webview names the Mac in its user agent, the Linux one (WebKitGTK) X11 and Linux.
 */
export function isMacUserAgent(userAgent: string): boolean {
  return /\bMacintosh\b|\bMac OS X\b/.test(userAgent);
}

/** True in the macOS app. */
export const isMac = typeof navigator !== 'undefined' && isMacUserAgent(navigator.userAgent);

export interface KeyHints {
  palette: string;
  /** `null` where the jump has no key. */
  nextNeeds: string | null;
}

/**
 * The shortcut hints the UI shows. Ctrl+K opens the palette on every platform. Cmd+J
 * has no Linux key: Ctrl+J is a newline in the session terminal.
 */
export function keyHints(mac: boolean): KeyHints {
  return mac ? { palette: '⌘K', nextNeeds: '⌘J' } : { palette: 'Ctrl+K', nextNeeds: null };
}

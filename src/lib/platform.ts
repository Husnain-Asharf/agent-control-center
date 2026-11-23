/** The system the app runs on. */
export type Platform = 'mac' | 'windows' | 'linux';

/**
 * Whether a webview's user agent is the macOS one. The app has no OS plugin; the
 * macOS webview names the Mac in its user agent, the Windows one (WebView2)
 * Windows NT, and the Linux one (WebKitGTK) X11 and Linux.
 */
export function isMacUserAgent(userAgent: string): boolean {
  return /\bMacintosh\b|\bMac OS X\b/.test(userAgent);
}

/** Whether a webview's user agent is the Windows one. */
export function isWindowsUserAgent(userAgent: string): boolean {
  return /\bWindows NT\b/.test(userAgent);
}

/** The platform a user agent names. Anything that is not macOS or Windows is Linux. */
export function platformOf(userAgent: string): Platform {
  if (isMacUserAgent(userAgent)) return 'mac';
  if (isWindowsUserAgent(userAgent)) return 'windows';
  return 'linux';
}

/** The platform of this app's webview. */
export const platform: Platform = typeof navigator !== 'undefined' ? platformOf(navigator.userAgent) : 'linux';

/** True in the macOS app. */
export const isMac = platform === 'mac';

export interface KeyHints {
  palette: string;
  /** `null` where the jump has no key. */
  nextNeeds: string | null;
  /** Text size up, down and back to 100%. */
  textSize: { bigger: string; smaller: string; reset: string };
}

/**
 * The shortcut hints the UI shows. Ctrl+K opens the palette on every platform. Cmd+J
 * has no key on Windows and Linux: Ctrl+J is a newline in the session terminal.
 * The text size keys are Cmd or Ctrl with =, - and 0.
 */
export function keyHints(on: Platform): KeyHints {
  const mod = on === 'mac' ? '⌘' : 'Ctrl+';
  const textSize = { bigger: `${mod}=`, smaller: `${mod}-`, reset: `${mod}0` };
  return on === 'mac'
    ? { palette: '⌘K', nextNeeds: '⌘J', textSize }
    : { palette: 'Ctrl+K', nextNeeds: null, textSize };
}

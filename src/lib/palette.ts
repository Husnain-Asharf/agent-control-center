const OPEN_EVENT = 'acc:open-palette';

/** Ask the app shell to open the command palette (the title bar search button uses this). */
export function openPalette(): void {
  window.dispatchEvent(new Event(OPEN_EVENT));
}

/** Calls `cb` whenever something asks for the palette. */
export function onOpenPalette(cb: () => void): () => void {
  window.addEventListener(OPEN_EVENT, cb);
  return () => window.removeEventListener(OPEN_EVENT, cb);
}

/** Command palette label for the sound toggle. */
export const soundActionLabel = (enabled: boolean): string => (enabled ? 'Turn sound off' : 'Turn sound on');

export interface DaemonStatus {
  connected: boolean;
  error: string | null;
}

/** Reads the `daemon://status` payload. An older daemon sends no `error`. */
export function parseStatusPayload(p: unknown): DaemonStatus {
  const o = p && typeof p === 'object' ? (p as Record<string, unknown>) : {};
  const error = typeof o.error === 'string' && o.error !== '' ? o.error : null;
  return { connected: o.connected === true, error };
}

export function daemonStatusText(connected: boolean, error: string | null): string {
  if (connected) return 'Daemon running';
  return error ? `Daemon not connected: ${error}` : 'Connecting to the daemon…';
}

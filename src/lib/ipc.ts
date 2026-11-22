import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { parseStatusPayload } from './daemonStatus';
import type { DaemonEvent, DaemonStatusPayload } from './types';

export class IpcError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.name = 'IpcError';
    this.code = code;
  }
}

function parseError(raw: unknown): IpcError {
  const text = typeof raw === 'string' ? raw : String(raw);
  try {
    const v = JSON.parse(text);
    if (v && typeof v.code === 'string' && typeof v.message === 'string') {
      return new IpcError(v.code, v.message);
    }
  } catch {
    // not JSON
  }
  return new IpcError('internal', text);
}

export async function request<T>(method: string, params?: object): Promise<T> {
  try {
    return await invoke<T>('daemon_request', { method, params: params ?? null });
  } catch (e) {
    throw parseError(e);
  }
}

export function onDaemonEvent(cb: (e: DaemonEvent) => void): () => void {
  return subscribe(listen<DaemonEvent>('daemon://event', (ev) => cb(ev.payload)));
}

export function onDaemonStatus(cb: (connected: boolean, error: string | null) => void): () => void {
  let live = true;
  let sawEvent = false;
  const unlisten = subscribe(
    listen<DaemonStatusPayload>('daemon://status', (ev) => {
      sawEvent = true;
      const s = parseStatusPayload(ev.payload);
      if (live) cb(s.connected, s.error);
    }),
  );
  // The first status event can fire before we subscribe, so ask once.
  invoke<boolean>('daemon_connected')
    .then((connected) => {
      if (live && !sawEvent) cb(connected, null);
    })
    .catch(() => {});
  return () => {
    live = false;
    unlisten();
  };
}

function subscribe(pending: Promise<() => void>): () => void {
  let cancelled = false;
  let unlisten: (() => void) | null = null;
  pending
    .then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    })
    .catch(() => {});
  return () => {
    cancelled = true;
    unlisten?.();
  };
}

// Terminal output goes straight to subscribers. It never enters the store.
/** `offset` is the stream position of the chunk's first byte. */
type Listener = (bytes: Uint8Array, offset: number) => void;

const listeners = new Map<string, Set<Listener>>();

export function publishOutput(sessionId: string, bytes: Uint8Array, offset: number): void {
  listeners.get(sessionId)?.forEach((cb) => cb(bytes, offset));
}

export function subscribeOutput(sessionId: string, cb: Listener): () => void {
  let set = listeners.get(sessionId);
  if (!set) listeners.set(sessionId, (set = new Set()));
  set.add(cb);
  return () => {
    set.delete(cb);
    if (set.size === 0) listeners.delete(sessionId);
  };
}

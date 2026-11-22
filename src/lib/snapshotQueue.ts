/**
 * Holds events that arrive while a snapshot request is in flight, so they can be
 * applied after the snapshot and not be lost or overwritten by it.
 * `begin` returns a token; a newer `begin` supersedes an older one.
 */
export function createReplayQueue<T>() {
  let token = 0;
  let open = false;
  let held: T[] = [];
  return {
    begin(): number {
      token += 1;
      open = true;
      held = [];
      return token;
    },
    /** True when the event was queued and the caller must not apply it now. */
    push(e: T): boolean {
      if (!open) return false;
      held.push(e);
      return true;
    },
    /** The queued events in arrival order, or none when a newer begin took over. */
    end(t: number): T[] {
      if (!open || t !== token) return [];
      open = false;
      const out = held;
      held = [];
      return out;
    },
    active: () => open,
  };
}

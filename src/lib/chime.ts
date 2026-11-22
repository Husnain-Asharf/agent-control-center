/** Minimum gap between two chimes, so a burst of requests sounds once. */
export const CHIME_GAP_MS = 1500;

/** Whether a chime should play now. `lastChimeAt` is the time of the previous one, or null. */
export function shouldChime(enabled: boolean, isNewPending: boolean, now: number, lastChimeAt: number | null): boolean {
  if (!enabled || !isNewPending) return false;
  return lastChimeAt === null || now - lastChimeAt >= CHIME_GAP_MS;
}

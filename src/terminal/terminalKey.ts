/** A restart (new started_at) or a reconnect (new epoch) gives a new key, so the terminal remounts. */
export const terminalKey = (session: { id: string; started_at: number }, connEpoch: number): string =>
  `${session.id}:${session.started_at}:${connEpoch}`;

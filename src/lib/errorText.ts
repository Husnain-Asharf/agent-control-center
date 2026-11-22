/** The message to show for a failed request: the daemon's own text when there is one. */
export function errorText(e: unknown): string {
  if (e instanceof Error && e.message) return e.message;
  if (typeof e === 'string' && e) return e;
  return 'Something went wrong.';
}

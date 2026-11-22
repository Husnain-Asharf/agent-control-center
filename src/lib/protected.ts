/**
 * True when the approval summary has a token that points at a protected file:
 * a basename starting with `.env`, ending in `.pem` or `.key`, or a path that
 * goes through a `.ssh` or `.aws` folder.
 */
export function touchesProtectedPath(summary: string): boolean {
  return summary
    .split(/\s+/)
    .map((t) => t.replace(/^["'`(]+|["'`),;]+$/g, ''))
    .filter(Boolean)
    .some((token) => {
      const parts = token.split('/');
      const base = parts[parts.length - 1];
      return (
        base.startsWith('.env') ||
        base.endsWith('.pem') ||
        base.endsWith('.key') ||
        parts.some((p) => p === '.ssh' || p === '.aws')
      );
    });
}

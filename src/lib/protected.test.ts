import { describe, expect, it } from 'vitest';
import { touchesProtectedPath } from './protected';

describe('touchesProtectedPath', () => {
  it.each(['.env', '.env.local', 'Edit config/.env.production', 'server.pem', 'id.key', 'certs/tls.key',
    '~/.ssh/id_rsa', '.ssh/id_rsa', '/Users/x/.aws/credentials', 'cat ~/.aws/credentials', 'cat "~/.ssh/config"'])(
    'flags %s',
    (s) => expect(touchesProtectedPath(s)).toBe(true),
  );

  it.each(['src/env.ts', 'README.md', 'npm install', 'src/keys.ts', 'Edit src/app.pem.md', 'git push origin main', ''])(
    'does not flag %s',
    (s) => expect(touchesProtectedPath(s)).toBe(false),
  );
});

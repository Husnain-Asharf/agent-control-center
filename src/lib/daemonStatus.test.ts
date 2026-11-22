import { describe, expect, it } from 'vitest';
import { daemonStatusText, parseStatusPayload } from './daemonStatus';

describe('daemonStatusText', () => {
  it('says running when connected', () => {
    expect(daemonStatusText(true, null)).toBe('Daemon running');
  });
  it('says connecting when disconnected with no error', () => {
    expect(daemonStatusText(false, null)).toBe('Connecting to the daemon…');
  });
  it('shows the error while disconnected', () => {
    expect(daemonStatusText(false, 'acc-hook was not found')).toBe('Daemon not connected: acc-hook was not found');
  });
  it('ignores an error when connected', () => {
    expect(daemonStatusText(true, 'stale')).toBe('Daemon running');
  });
});

describe('parseStatusPayload', () => {
  it('reads connected and error', () => {
    expect(parseStatusPayload({ connected: false, error: 'boom' })).toEqual({ connected: false, error: 'boom' });
    expect(parseStatusPayload({ connected: true })).toEqual({ connected: true, error: null });
  });
  it('drops an empty or non-string error', () => {
    expect(parseStatusPayload({ connected: false, error: '' })).toEqual({ connected: false, error: null });
    expect(parseStatusPayload({ connected: false, error: 3 })).toEqual({ connected: false, error: null });
  });
  it('treats a malformed payload as disconnected', () => {
    expect(parseStatusPayload(null)).toEqual({ connected: false, error: null });
  });
});

import { describe, expect, it } from 'vitest';
import { CHIME_GAP_MS, shouldChime } from './chime';

describe('shouldChime', () => {
  it('plays for a new pending approval when enabled and nothing played yet', () => {
    expect(shouldChime(true, true, 1000, null)).toBe(true);
  });
  it('stays silent when sound is off', () => {
    expect(shouldChime(false, true, 1000, null)).toBe(false);
  });
  it('stays silent for replayed or already pending approvals', () => {
    expect(shouldChime(true, false, 1000, null)).toBe(false);
  });
  it('plays at most once per gap', () => {
    expect(shouldChime(true, true, 1000 + CHIME_GAP_MS - 1, 1000)).toBe(false);
    expect(shouldChime(true, true, 1000 + CHIME_GAP_MS, 1000)).toBe(true);
  });
});

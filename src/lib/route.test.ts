import { describe, expect, it } from 'vitest';
import { parseHash } from './route';

describe('parseHash', () => {
  it('reads a screen and its parameter', () => {
    expect(parseHash('#settings')).toEqual({ name: 'settings', param: null });
    expect(parseHash('#session/a%2Fb')).toEqual({ name: 'session', param: 'a/b' });
  });
  it('falls back to the Overview', () => {
    expect(parseHash('')).toEqual({ name: 'overview', param: null });
    expect(parseHash('#nope')).toEqual({ name: 'overview', param: null });
  });
});

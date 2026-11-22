import { describe, expect, it } from 'vitest';
import { soundActionLabel } from './palette';

describe('soundActionLabel', () => {
  it('offers the opposite of the current state', () => {
    expect(soundActionLabel(true)).toBe('Turn sound off');
    expect(soundActionLabel(false)).toBe('Turn sound on');
  });
});

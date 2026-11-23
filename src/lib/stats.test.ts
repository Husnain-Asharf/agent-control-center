import { describe, expect, it } from 'vitest';
import { fmtBytes } from './stats';

describe('fmtBytes', () => {
  it('writes sizes in KB, MB and GB', () => {
    expect(fmtBytes(null)).toBeNull();
    expect(fmtBytes(10)).toBe('1 KB');
    expect(fmtBytes(512 * 1024)).toBe('512 KB');
    expect(fmtBytes(3.4 * 1024 * 1024)).toBe('3.4 MB');
    expect(fmtBytes(1.25 * 1024 ** 3)).toBe('1.3 GB');
  });
});

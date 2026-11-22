import { describe, expect, it } from 'vitest';
import { b64ToBytes, bytesToB64 } from './b64';

describe('b64', () => {
  it('round-trips UTF-8 and binary', () => {
    const text = new TextEncoder().encode('→ ok ✓');
    expect(new TextDecoder().decode(b64ToBytes(bytesToB64(text)))).toBe('→ ok ✓');

    const all = Uint8Array.from({ length: 256 }, (_, i) => i);
    expect(Array.from(b64ToBytes(bytesToB64(all)))).toEqual(Array.from(all));
  });

  it('handles empty and large input', () => {
    expect(b64ToBytes(bytesToB64(new Uint8Array(0))).length).toBe(0);
    const big = new Uint8Array(200_000).map((_, i) => i % 251);
    expect(b64ToBytes(bytesToB64(big))).toEqual(big);
  });
});

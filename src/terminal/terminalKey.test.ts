import { describe, expect, it } from 'vitest';
import { terminalKey } from './terminalKey';

describe('terminalKey', () => {
  const s = { id: 'a', started_at: 100 };
  it('changes on restart and on reconnect only', () => {
    expect(terminalKey(s, 1)).toBe(terminalKey({ ...s }, 1));
    expect(terminalKey(s, 1)).not.toBe(terminalKey({ ...s, started_at: 200 }, 1));
    expect(terminalKey(s, 1)).not.toBe(terminalKey(s, 2));
    expect(terminalKey(s, 1)).not.toBe(terminalKey({ ...s, id: 'b' }, 1));
  });
});

import { describe, expect, it } from 'vitest';
import { OutputSplicer } from './splice';

const b = (s: string) => new TextEncoder().encode(s);
function run() {
  const out: string[] = [];
  const write = (x: Uint8Array) => out.push(new TextDecoder().decode(x));
  return { out, write, s: new OutputSplicer() };
}

describe('OutputSplicer', () => {
  it('skips a chunk entirely before end', () => {
    const { out, write, s } = run();
    s.attach(10, write);
    s.push(b('abcde'), 3, write);
    expect(out).toEqual([]);
  });
  it('writes the tail of a straddling chunk', () => {
    const { out, write, s } = run();
    s.attach(5, write);
    s.push(b('abcdef'), 2, write);
    expect(out).toEqual(['def']);
  });
  it('writes a chunk after end whole', () => {
    const { out, write, s } = run();
    s.attach(5, write);
    s.push(b('xyz'), 5, write);
    expect(out).toEqual(['xyz']);
  });
  it('writes a gap chunk whole and jumps end', () => {
    const { out, write, s } = run();
    s.attach(5, write);
    s.push(b('xyz'), 20, write);
    s.push(b('q'), 23, write);
    s.push(b('old'), 5, write);
    expect(out).toEqual(['xyz', 'q']);
  });
  it('queues before attach, then flushes only bytes past end', () => {
    const { out, write, s } = run();
    s.push(b('0123'), 0, write);
    s.push(b('4567'), 4, write);
    s.push(b('89'), 8, write);
    expect(out).toEqual([]);
    s.attach(6, write);
    s.push(b('ab'), 10, write);
    expect(out).toEqual(['67', '89', 'ab']);
  });
  it('advances end across a sequence of chunks', () => {
    const { out, write, s } = run();
    s.attach(0, write);
    s.push(b('ab'), 0, write);
    s.push(b('cd'), 2, write);
    s.push(b('cd'), 2, write);
    s.push(b('ef'), 4, write);
    expect(out.join('')).toBe('abcdef');
  });
});

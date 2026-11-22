import { describe, expect, it } from 'vitest';
import { errorText } from './errorText';

describe('errorText', () => {
  it('uses the message of an Error, such as an IpcError with a hook_not_found message', () => {
    const e = Object.assign(new Error('acc-hook was not found at /x. Run cargo build --workspace, then pkill accd and reopen the app.'), {
      code: 'hook_not_found',
    });
    expect(errorText(e)).toBe('acc-hook was not found at /x. Run cargo build --workspace, then pkill accd and reopen the app.');
  });
  it('falls back to a string or a generic line', () => {
    expect(errorText('plain')).toBe('plain');
    expect(errorText(undefined)).toBe('Something went wrong.');
    expect(errorText(new Error(''))).toBe('Something went wrong.');
  });
});

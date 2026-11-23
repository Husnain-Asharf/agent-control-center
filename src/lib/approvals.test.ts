import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ONCE_ONLY_NOTE,
  alwaysAllowState,
  approvalKey,
  decide,
  branchQuestionBase,
  decisionFor,
  decisionKeyAllowed,
  decisionKeysReady,
  isBranchQuestion,
  isProtectedApproval,
  wantsText,
  wantsVerb,
} from './approvals';
import { useStore } from '../store/store';

const mockRequest = vi.fn();
vi.mock('./ipc', () => ({ request: (...a: unknown[]) => mockRequest(...a) }));

const key = (k: string, extra: Record<string, unknown> = {}) => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  target: null,
  ...extra,
});

beforeEach(() => {
  useStore.setState(useStore.getInitialState(), true);
  mockRequest.mockReset();
});

describe('wantsText', () => {
  it('names the kind of request', () => {
    expect(wantsText('Bash')).toBe('Wants to run a command');
    expect(wantsText('PowerShell')).toBe('Wants to run a command');
    expect(wantsText('Edit')).toBe('Wants to edit a file');
    expect(wantsText('Write')).toBe('Wants to edit a file');
    expect(wantsText('WebFetch')).toBe('Wants to use WebFetch');
  });
});

describe('wantsVerb', () => {
  it('uses run only for commands', () => {
    expect(wantsVerb('Bash')).toBe('run');
    expect(wantsVerb('PowerShell')).toBe('run');
    expect(wantsVerb('Edit')).toBe('edit');
    expect(wantsVerb('MultiEdit')).toBe('edit');
    expect(wantsVerb('Write')).toBe('edit');
    expect(wantsVerb('WebFetch')).toBe('use WebFetch');
  });
});

describe('branchQuestionBase', () => {
  it('names the folder’s branch, or says no branch', () => {
    expect(branchQuestionBase({ on_branch: 'main' }, 'main')).toBe('main');
    expect(branchQuestionBase({ on_branch: null }, 'main')).toBe('no branch');
    expect(branchQuestionBase({}, 'main')).toBe('main');
  });
});

describe('decisionFor', () => {
  it('keeps the decision for a permission request', () => {
    for (const d of ['allow_once', 'allow_always', 'deny'] as const) {
      expect(decisionFor({}, d)).toBe(d);
      expect(decisionFor({ kind: 'permission' }, d)).toBe(d);
    }
  });
  it('maps the same keys to the branch answers on a branch question', () => {
    const q = { kind: 'branch' as const };
    expect(isBranchQuestion(q)).toBe(true);
    expect(decisionFor(q, 'allow_once')).toBe('make_branch');
    expect(decisionFor(q, 'allow_always')).toBe('stay_on_base');
    expect(decisionFor(q, 'deny')).toBe('deny');
  });
});

describe('approvalKey', () => {
  it('maps the approval shortcuts', () => {
    expect(approvalKey(key('a'))).toBe('allow_once');
    expect(approvalKey(key('A', { shiftKey: true }))).toBe('allow_always');
    expect(approvalKey(key('d'))).toBe('deny');
    expect(approvalKey(key('j'))).toBe('next');
    expect(approvalKey(key('k'))).toBe('prev');
  });

  it('ignores repeated decision keys but not a fresh press', () => {
    expect(approvalKey(key('a', { repeat: true }))).toBeNull();
    expect(approvalKey(key('A', { shiftKey: true, repeat: true }))).toBeNull();
    expect(approvalKey(key('d', { repeat: true }))).toBeNull();
    expect(approvalKey(key('a', { repeat: false }))).toBe('allow_once');
    expect(approvalKey(key('j', { repeat: true }))).toBe('next');
  });

  it('ignores other keys and modified keys', () => {
    expect(approvalKey(key('x'))).toBeNull();
    expect(approvalKey(key('a', { metaKey: true }))).toBeNull();
    expect(approvalKey(key('d', { ctrlKey: true }))).toBeNull();
    expect(approvalKey(key('j', { altKey: true }))).toBeNull();
    expect(approvalKey(key('a', { defaultPrevented: true }))).toBeNull();
  });

  it('is skipped while typing', () => {
    expect(approvalKey(key('a', { target: { tagName: 'INPUT' } }))).toBeNull();
    expect(approvalKey(key('d', { target: { tagName: 'TEXTAREA' } }))).toBeNull();
    expect(approvalKey(key('j', { target: { closest: () => ({}) } }))).toBeNull();
  });
});

describe('decide', () => {
  it('sends the snake_case decision', async () => {
    mockRequest.mockResolvedValue({});
    await decide('a1', 'allow_once');
    expect(mockRequest).toHaveBeenCalledWith('approval_decide', { approval_id: 'a1', decision: 'allow_once' });
    expect(useStore.getState().toasts).toHaveLength(0);
  });

  it('treats already_resolved as success', async () => {
    mockRequest.mockResolvedValue({ already_resolved: true });
    await decide('a1', 'deny');
    expect(useStore.getState().toasts).toHaveLength(0);
  });

  it('shows an error toast when the request fails', async () => {
    mockRequest.mockRejectedValue(new Error('Not connected to the daemon.'));
    await decide('a1', 'deny');
    const toasts = useStore.getState().toasts;
    expect(toasts).toHaveLength(1);
    expect(toasts[0].msg).toBe('Not connected to the daemon.');
  });

  it('sends one request for concurrent decides on the same id', async () => {
    let finish: (v: unknown) => void = () => {};
    mockRequest.mockReturnValue(new Promise((r) => (finish = r)));
    const first = decide('a1', 'allow_once');
    await decide('a1', 'allow_once');
    expect(mockRequest).toHaveBeenCalledTimes(1);
    finish({});
    await first;
  });

  it('sends again once the first decide has settled', async () => {
    mockRequest.mockResolvedValue({});
    await decide('a1', 'deny');
    await decide('a1', 'deny');
    expect(mockRequest).toHaveBeenCalledTimes(2);
  });
});

describe('protected approvals', () => {
  const put = (id: string, summary: string, extra: Record<string, unknown> = {}) =>
    useStore.setState({
      approvals: {
        [id]: { id, session_id: 's', tool: 'Edit', summary, input: null, created_at: 1, state: 'pending', decided_by: null, ...extra },
      },
    });

  it('isProtectedApproval trusts the field and falls back to the summary when it is missing', () => {
    expect(isProtectedApproval({ summary: 'npm install', protected: true })).toBe(true);
    expect(isProtectedApproval({ summary: 'Edit .env', protected: false })).toBe(false);
    expect(isProtectedApproval({ summary: 'Edit .env' })).toBe(true);
    expect(isProtectedApproval({ summary: 'npm install' })).toBe(false);
  });

  it('alwaysAllowState shows the rule preview and enables the button', () => {
    expect(alwaysAllowState({ summary: 'npm install', protected: false, rule_preview: '`npm install` commands' })).toEqual({
      enabled: true,
      preview: '`npm install` commands',
      note: null,
    });
  });

  it('alwaysAllowState disables the button with the protected note', () => {
    expect(alwaysAllowState({ summary: 'Edit .env', protected: true, rule_preview: null })).toEqual({
      enabled: false,
      preview: null,
      note: 'Protected files can never be always allowed',
    });
  });

  it('alwaysAllowState disables the button with the once-only note', () => {
    expect(alwaysAllowState({ summary: 'bash -c x', protected: false, rule_preview: null })).toEqual({
      enabled: false,
      preview: null,
      note: ONCE_ONLY_NOTE,
    });
    expect(ONCE_ONLY_NOTE).toBe('This request can only be allowed once');
  });

  it('alwaysAllowState treats missing fields as no preview, with the summary fallback for protected', () => {
    expect(alwaysAllowState({ summary: 'npm install' }).enabled).toBe(false);
    expect(alwaysAllowState({ summary: 'npm install' }).note).toBe(ONCE_ONLY_NOTE);
    expect(alwaysAllowState({ summary: 'Edit .env' }).note).toBe('Protected files can never be always allowed');
  });

  it('decide refuses allow_always on a protected item and tells the user', async () => {
    put('p1', 'Edit .env', { protected: true, rule_preview: null });
    await decide('p1', 'allow_always');
    expect(mockRequest).not.toHaveBeenCalled();
    expect(useStore.getState().toasts[0].msg).toBe('Protected files can never be always allowed.');
  });

  it('decide refuses allow_always when rule_preview is null', async () => {
    put('p3', 'bash -c x', { protected: false, rule_preview: null });
    await decide('p3', 'allow_always');
    expect(mockRequest).not.toHaveBeenCalled();
    expect(useStore.getState().toasts[0].msg).toBe('This request can only be allowed once.');
  });

  it('decide still allows once and denies a protected item', async () => {
    mockRequest.mockResolvedValue({});
    put('p1', 'Edit .env', { protected: true, rule_preview: null });
    await decide('p1', 'allow_once');
    await decide('p1', 'deny');
    expect(mockRequest).toHaveBeenCalledTimes(2);
  });

  it('decide allows always when there is a rule preview', async () => {
    mockRequest.mockResolvedValue({});
    put('p2', 'npm install', { protected: false, rule_preview: '`npm install` commands' });
    await decide('p2', 'allow_always');
    expect(mockRequest).toHaveBeenCalledTimes(1);
  });
});

describe('decisionKeyAllowed', () => {
  it('ignores decision keys while a dialog is open', () => {
    expect(decisionKeyAllowed('allow_once', true)).toBe(false);
    expect(decisionKeyAllowed('allow_always', true)).toBe(false);
    expect(decisionKeyAllowed('deny', true)).toBe(false);
  });
  it('allows decision keys with no dialog, and navigation keys always', () => {
    expect(decisionKeyAllowed('allow_once', false)).toBe(true);
    expect(decisionKeyAllowed('deny', false)).toBe(true);
    expect(decisionKeyAllowed('next', true)).toBe(true);
    expect(decisionKeyAllowed('prev', false)).toBe(true);
  });
});

describe('decisionKeysReady', () => {
  it('waits 400 ms after the selection changes', () => {
    expect(decisionKeysReady(1399, 1000)).toBe(false);
    expect(decisionKeysReady(1400, 1000)).toBe(true);
  });

  it('starts again when the selection changes', () => {
    expect(decisionKeysReady(2000, 1000)).toBe(true);
    expect(decisionKeysReady(2000, 1900)).toBe(false);
  });
});

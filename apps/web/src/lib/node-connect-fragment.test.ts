import { describe, expect, it, vi } from 'vitest';
import { consumeNodeConnectFragment } from './node-connect-fragment';

describe('consumeNodeConnectFragment', () => {
  it('scrubs a valid one-time token from the address bar and keeps it in memory', () => {
    const replaceState = vi.fn();
    const token = `piq_enr_${'a'.repeat(32)}`;

    expect(
      consumeNodeConnectFragment(
        { hash: `#enrolment_token=${token}`, pathname: '/downloads', search: '?platform=macos' },
        (url) => replaceState(null, '', url)
      )
    ).toEqual({ enrolmentToken: token });
    expect(replaceState).toHaveBeenCalledWith(null, '', '/downloads?platform=macos');
  });

  it('scrubs malformed capabilities instead of retaining or exposing them', () => {
    const replaceState = vi.fn();
    expect(
      consumeNodeConnectFragment(
        { hash: '#enrolment_token=not-a-token', pathname: '/downloads', search: '' },
        (url) => replaceState(null, '', url)
      )
    ).toBeNull();
    expect(replaceState).toHaveBeenCalledWith(null, '', '/downloads');
  });

  it('leaves unrelated fragments intact', () => {
    const replaceState = vi.fn();
    expect(
      consumeNodeConnectFragment(
        { hash: '#other-downloads', pathname: '/downloads', search: '' },
        (url) => replaceState(null, '', url)
      )
    ).toBeNull();
    expect(replaceState).not.toHaveBeenCalled();
  });
});

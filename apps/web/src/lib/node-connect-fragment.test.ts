import { describe, expect, it, vi } from 'vitest';
import { consumeNodeConnectFragment, nativeNodeConnectUrl } from './node-connect-fragment';

describe('consumeNodeConnectFragment', () => {
  it('scrubs a valid one-time token from the address bar and keeps it in memory', () => {
    const replaceState = vi.fn();
    const token = `piq_enr_${'a'.repeat(32)}`;
    const origin = 'https://api.example.com';

    expect(
      consumeNodeConnectFragment(
        { hash: `#enrolment_token=${token}&control_plane_url=${encodeURIComponent(origin)}`, pathname: '/downloads', search: '?platform=macos' },
        (url) => replaceState(null, '', url)
      )
    ).toEqual({ enrolmentToken: token, controlPlaneUrl: origin });
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

describe('nativeNodeConnectUrl', () => {
  it('keeps a validated invitation in the fragment of the registered app scheme', () => {
    const token = `piq_enr_${'a'.repeat(32)}`;
    expect(nativeNodeConnectUrl(token, 'https://api.example.com')).toBe(`piqae://connect#enrolment_token=${token}&control_plane_url=https%3A%2F%2Fapi.example.com`);
  });

  it('refuses malformed capabilities', () => {
    expect(nativeNodeConnectUrl('not-a-capability', 'https://api.example.com')).toBeNull();
  });
});

import { describe, expect, it } from 'vitest';
import { isSameOriginRequest } from './same-origin';

const url = new URL('https://app.piqae.com/auth/switch');

describe('isSameOriginRequest', () => {
  it('accepts only the exact scheme and host origin', () => {
    const request = new Request(url, { headers: { origin: 'https://app.piqae.com' } });
    expect(isSameOriginRequest(request, url)).toBe(true);
  });

  it.each([
    [undefined],
    ['https://attacker.example'],
    ['http://app.piqae.com'],
    ['https://app.piqae.com.attacker.example'],
    ['not an origin']
  ])('rejects a missing or cross-origin mutation origin: %s', (origin) => {
    const headers = origin ? { origin } : undefined;
    expect(isSameOriginRequest(new Request(url, { headers }), url)).toBe(false);
  });
});

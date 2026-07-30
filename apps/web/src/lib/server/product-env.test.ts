import { describe, expect, it } from 'vitest';
import { productEnvironmentValue } from './product-env';

describe('product environment migration', () => {
  it('prefers the Piqae variable, including an intentional empty value', () => {
    expect(
      productEnvironmentValue(
        { PIQAE_AUTH_MODE: '', SPOOL_AUTH_MODE: 'workos' },
        'PIQAE_AUTH_MODE'
      )
    ).toBe('');
  });

  it('accepts private and public legacy variables through V1', () => {
    expect(
      productEnvironmentValue({ SPOOL_AUTH_MODE: 'workos' }, 'PIQAE_AUTH_MODE')
    ).toBe('workos');
    expect(
      productEnvironmentValue(
        { PUBLIC_SPOOL_API_URL: 'https://legacy.example.test' },
        'PUBLIC_PIQAE_API_URL'
      )
    ).toBe('https://legacy.example.test');
  });
});

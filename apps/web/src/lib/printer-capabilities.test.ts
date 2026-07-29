import { describe, expect, it } from 'vitest';
import { copyLimit } from './printer-capabilities';

describe('copyLimit', () => {
  it('treats a CUPS zero as an unspecified limit', () => {
    expect(copyLimit(0)).toBe(99);
  });

  it('preserves a positive driver-provided limit', () => {
    expect(copyLimit(12)).toBe(12);
  });

  it('falls back for missing and invalid values', () => {
    expect(copyLimit(undefined)).toBe(99);
    expect(copyLimit(Number.NaN)).toBe(99);
  });
});

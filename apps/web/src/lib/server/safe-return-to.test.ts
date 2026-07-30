import { describe, expect, it } from 'vitest';
import { safeReturnTo } from './safe-return-to';

describe('safeReturnTo', () => {
  it.each([
    ['https://attacker.example', '/dashboard'],
    ['//attacker.example', '/dashboard'],
    ['/\\attacker.example', '/dashboard'],
    ['not-a-path', '/dashboard']
  ])('rejects an external redirect form: %s', (candidate, expected) => {
    expect(safeReturnTo(candidate)).toBe(expected);
  });

  it('preserves a normalized same-origin path, query, and fragment', () => {
    expect(safeReturnTo('/dashboard/../dashboard/jobs?state=queued#latest')).toBe(
      '/dashboard/jobs?state=queued#latest'
    );
  });
});

import { describe, expect, it } from 'vitest';
import { dashboardNavigation } from './dashboard-navigation';

describe('dashboard navigation capabilities', () => {
  it('shows Customers only when platform accounts are explicitly enabled', () => {
    expect(
      dashboardNavigation({ platform: { accounts: true } }).map((item) => item.label)
    ).toContain('Customers');
    expect(
      dashboardNavigation({ platform: { accounts: false } }).map((item) => item.label)
    ).not.toContain('Customers');
  });
});

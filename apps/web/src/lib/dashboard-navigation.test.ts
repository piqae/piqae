import { describe, expect, it } from 'vitest';
import {
  dashboardNavigation,
  isOperationalView,
  operationalViews
} from './dashboard-navigation';

describe('dashboard navigation capabilities', () => {
  it('shows Customers only when platform accounts are explicitly enabled', () => {
    expect(
      operationalViews({ platform: { accounts: true } }).map((view) => view.label)
    ).toContain('Customers');
    expect(
      operationalViews({ platform: { accounts: false } }).map((view) => view.label)
    ).not.toContain('Customers');
  });

  it('rejects a customers view on deployments without the accounts feature', () => {
    expect(isOperationalView('customers', { platform: { accounts: true } })).toBe(true);
    expect(isOperationalView('customers', { platform: { accounts: false } })).toBe(false);
    expect(isOperationalView('nonsense', { platform: { accounts: true } })).toBe(false);
  });

  it('keeps the dashboard to two destinations', () => {
    expect(dashboardNavigation({ platform: { accounts: true } }).map((item) => item.href)).toEqual([
      '/dashboard',
      '/dashboard/settings'
    ]);
  });
});

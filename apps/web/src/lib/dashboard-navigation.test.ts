import { describe, expect, it } from 'vitest';
import {
  dashboardNavigation,
  isOperationalView,
  isStateFilter,
  operationalViews,
  resolveStateFilter,
  stateFilters
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

describe('operational state filters', () => {
  it('offers uncertain delivery as a first-class job filter', () => {
    expect(stateFilters('jobs')).toContainEqual({ value: 'delivery_uncertain', label: 'Uncertain' });
    expect(isStateFilter('delivery_uncertain', 'jobs')).toBe(true);
  });

  it('keeps job states out of the printer and node filters', () => {
    expect(isStateFilter('delivery_uncertain', 'printers')).toBe(false);
    expect(isStateFilter('online', 'nodes')).toBe(true);
  });

  it('offers no state filter for customers', () => {
    expect(stateFilters('customers')).toEqual([]);
    expect(isStateFilter('online', 'customers')).toBe(false);
  });

  it('widens an unknown or inapplicable state to everything', () => {
    expect(resolveStateFilter('delivery_uncertain', 'jobs')).toBe('delivery_uncertain');
    expect(resolveStateFilter('delivery_uncertain', 'printers')).toBe('all');
    expect(resolveStateFilter('nonsense', 'jobs')).toBe('all');
    expect(resolveStateFilter(null, 'jobs')).toBe('all');
  });
});

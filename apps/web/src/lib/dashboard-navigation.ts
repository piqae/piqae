import type { DashboardMeta } from './view-types';

/**
 * The dashboard has two destinations: the operational surface and everything
 * that configures it. Jobs, printers and nodes are views within `/dashboard`
 * rather than routes of their own — see `operationalViews`.
 */
export function dashboardNavigation(_meta: Pick<DashboardMeta, 'platform'>) {
  return [
    { href: '/dashboard', label: 'Operations', icon: 'activity' },
    { href: '/dashboard/settings', label: 'Settings', icon: 'settings' }
  ] as const;
}

export type OperationalView = 'jobs' | 'printers' | 'nodes' | 'customers';

/**
 * View switcher options for `/dashboard?view=`. Customers only appears on
 * deployments where the accounts platform feature is enabled.
 */
export function operationalViews(meta: Pick<DashboardMeta, 'platform'>) {
  return [
    { value: 'jobs', label: 'Jobs' },
    { value: 'printers', label: 'Printers' },
    { value: 'nodes', label: 'Nodes' },
    ...(meta.platform.accounts ? [{ value: 'customers', label: 'Customers' }] : [])
  ];
}

export function isOperationalView(
  value: string | null,
  meta: Pick<DashboardMeta, 'platform'>
): value is OperationalView {
  return operationalViews(meta).some((view) => view.value === value);
}

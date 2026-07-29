import type { DashboardMeta } from './view-types';

export function dashboardNavigation(meta: Pick<DashboardMeta, 'platform'>) {
  return [
    { href: '/dashboard', label: 'Overview', icon: 'activity' },
    { href: '/dashboard/jobs', label: 'Jobs', icon: 'jobs' },
    { href: '/dashboard/printers', label: 'Printers', icon: 'printers' },
    { href: '/dashboard/nodes', label: 'Nodes', icon: 'agents' },
    ...(meta.platform.accounts
      ? [{ href: '/dashboard/accounts', label: 'Customers', icon: 'agents' } as const]
      : []),
    { href: '/dashboard/developers', label: 'Developers', icon: 'api' }
  ] as const;
}

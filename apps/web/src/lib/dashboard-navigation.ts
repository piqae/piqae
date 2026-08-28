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

export type OperationalView =
  | 'jobs'
  | 'queue'
  | 'destinations'
  | 'routes'
  | 'needs_review'
  | 'printers'
  | 'nodes'
  | 'customers';

export type PrimaryOperationalView = 'jobs' | 'printers' | 'nodes' | 'customers';

/**
 * Primary view switcher options for `/dashboard?view=`. Queue telemetry is a
 * Jobs subview; physical destinations and routes are advanced printer
 * diagnostics. Those addresses remain valid but do not compete with the four
 * resources operators use every day. Customers only appears when the accounts
 * platform feature is enabled.
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
  if (value === 'customers') return meta.platform.accounts;
  return [
    'jobs',
    'queue',
    'destinations',
    'routes',
    'needs_review',
    'printers',
    'nodes'
  ].includes(value ?? '');
}

/** The primary tab that owns an operational subview or diagnostic address. */
export function primaryOperationalView(view: OperationalView): PrimaryOperationalView {
  if (view === 'queue' || view === 'needs_review') return 'jobs';
  if (view === 'destinations' || view === 'routes') return 'printers';
  return view;
}

const JOB_STATE_FILTERS = [
  { value: 'all', label: 'All states' },
  { value: 'active', label: 'Active' },
  { value: 'failed', label: 'Failed' },
  { value: 'delivery_uncertain', label: 'Uncertain' }
] as const;

const RESOURCE_STATE_FILTERS = [
  { value: 'all', label: 'All states' },
  { value: 'online', label: 'Online' },
  { value: 'degraded', label: 'Degraded' },
  { value: 'offline', label: 'Offline' },
  { value: 'paused', label: 'Paused' }
] as const;

const ROUTE_HEALTH_FILTERS = [
  { value: 'all', label: 'All route health' },
  { value: 'ready', label: 'Ready' },
  { value: 'busy', label: 'Busy' },
  { value: 'needs_operator', label: 'Needs operator' },
  { value: 'offline', label: 'Offline' },
  { value: 'stale', label: 'Stale' },
  { value: 'unknown', label: 'Unknown' }
] as const;

export interface StateFilterOption {
  value: string;
  label: string;
}

/**
 * State narrowing for `/dashboard?view=…&state=…`. The filter lives in the
 * query string rather than component state so a link can land an operator on
 * exactly one set of jobs — `state=delivery_uncertain` above all, which is the
 * only way a non-zero uncertain count leads anywhere. Customers carry no
 * operational state, so they offer no filter.
 */
export function stateFilters(view: OperationalView): StateFilterOption[] {
  if (view === 'customers' || view === 'queue' || view === 'destinations' || view === 'needs_review') return [];
  if (view === 'routes') return [...ROUTE_HEALTH_FILTERS];
  return [...(view === 'jobs' ? JOB_STATE_FILTERS : RESOURCE_STATE_FILTERS)];
}

export function isStateFilter(value: string | null, view: OperationalView): boolean {
  return stateFilters(view).some((filter) => filter.value === value);
}

/** An unknown or inapplicable `state=` must widen to everything, never hide rows. */
export function resolveStateFilter(value: string | null, view: OperationalView): string {
  return isStateFilter(value, view) ? (value as string) : 'all';
}

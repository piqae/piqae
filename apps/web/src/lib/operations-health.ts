import type {
  DashboardDestination,
  DashboardPrinter,
  DashboardPrinterRoute
} from './view-types';

export function printerNeedsAttention(state: string): boolean {
  return ['degraded', 'error', 'offline', 'paper_out', 'paused'].includes(state);
}

export function destinationNeedsReview(destination: DashboardDestination): boolean {
  return destination.status === 'needs_review' || destination.identityConfidence === 'conflict';
}

export function routeNeedsReview(route: DashboardPrinterRoute): boolean {
  return route.health === 'needs_operator'
    || route.health === 'offline'
    || route.health === 'stale'
    || route.projectionHealth === 'failed'
    || route.telemetryFreshness === 'stale';
}

export function identitySummary(destination: DashboardDestination): string {
  switch (destination.identityConfidence) {
    case 'verified': return 'Verified';
    case 'high': return 'High confidence';
    case 'conflict': return 'Conflict';
    case 'possible': return destination.routeCount === 1
      ? 'Provisional · 1 route'
      : `Possible match · ${destination.routeCount} routes`;
    case 'unknown': return destination.routeCount === 1
      ? 'Provisional · 1 route'
      : `Unverified · ${destination.routeCount} routes`;
  }
}

export function routeHealthSummary(route: DashboardPrinterRoute): string {
  if (route.telemetryFreshness === 'never') return 'Limited telemetry';
  if (route.health === 'unknown') return 'Limited telemetry';
  return route.health.replaceAll('_', ' ');
}

export function profilesSummary(
  route: DashboardPrinterRoute,
  printer: DashboardPrinter | undefined
): string {
  if (!route.profileObservedAt) return 'Not reported';
  if (!printer) return 'Reported';
  const ready = printer.profiles.filter((profile) => profile.status === 'ready').length;
  return `${ready}/${printer.profiles.length} ready`;
}

export function stockSummary(route: DashboardPrinterRoute): string {
  if (!route.stockObservedAt || route.stockState === 'not_reported') return 'Not reported';
  return route.stockState.replaceAll('_', ' ');
}

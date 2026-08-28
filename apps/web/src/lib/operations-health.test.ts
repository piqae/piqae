import { describe, expect, it } from 'vitest';
import {
  destinationNeedsReview,
  identitySummary,
  printerNeedsAttention,
  profilesSummary,
  routeHealthSummary,
  routeNeedsReview,
  stockSummary
} from './operations-health';
import type { DashboardDestination, DashboardPrinterRoute } from './view-types';

const destination = (overrides: Partial<DashboardDestination> = {}): DashboardDestination => ({
  id: 'pdst_test',
  displayName: 'Dispatch printer',
  manufacturer: null,
  model: null,
  identityConfidence: 'unknown',
  status: 'active',
  routeCount: 1,
  updatedAt: '2026-08-27T00:00:00Z',
  ...overrides
});

const route = (overrides: Partial<DashboardPrinterRoute> = {}): DashboardPrinterRoute => ({
  id: 'rte_test',
  physicalDestinationId: 'pdst_test',
  printerId: 'prt_test',
  agentId: 'agt_test',
  nativeQueueId: 'Dispatch',
  enabled: true,
  health: 'unknown',
  telemetryFreshness: 'never',
  projectionHealth: 'current',
  capabilityRevision: 1,
  profileRevision: 1,
  profileObservedAt: null,
  stockObservedAt: null,
  stockState: 'not_reported',
  schedulingAuthorityId: null,
  latestObservation: null,
  updatedAt: '2026-08-27T00:00:00Z',
  ...overrides
});

describe('operations health language', () => {
  it('does not turn busy or unreported printer state into an incident', () => {
    expect(printerNeedsAttention('busy')).toBe(false);
    expect(printerNeedsAttention('unknown')).toBe(false);
    expect(printerNeedsAttention('paused')).toBe(true);
    expect(printerNeedsAttention('paper_out')).toBe(true);
  });

  it('keeps a single unverified route provisional rather than actionable', () => {
    const value = destination();
    expect(identitySummary(value)).toBe('Provisional · 1 route');
    expect(destinationNeedsReview(value)).toBe(false);
  });

  it('only promotes explicit identity conflicts into review', () => {
    expect(destinationNeedsReview(destination({ identityConfidence: 'conflict' }))).toBe(true);
    expect(destinationNeedsReview(destination({ status: 'needs_review' }))).toBe(true);
  });

  it('separates limited telemetry, profiles, and unreported stock', () => {
    const value = route();
    expect(routeHealthSummary(value)).toBe('Limited telemetry');
    expect(profilesSummary(value, undefined)).toBe('Not reported');
    expect(stockSummary(value)).toBe('Not reported');
    expect(routeNeedsReview(value)).toBe(false);
  });

  it('reviews stale, offline, operator, and rejected projection routes', () => {
    expect(routeNeedsReview(route({ health: 'stale', telemetryFreshness: 'stale' }))).toBe(true);
    expect(routeNeedsReview(route({ health: 'offline', telemetryFreshness: 'recent' }))).toBe(true);
    expect(routeNeedsReview(route({ health: 'needs_operator', telemetryFreshness: 'live' }))).toBe(true);
    expect(routeNeedsReview(route({ projectionHealth: 'failed' }))).toBe(true);
  });
});

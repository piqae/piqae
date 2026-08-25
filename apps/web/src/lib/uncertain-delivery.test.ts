import { describe, expect, it } from 'vitest';
import {
  summariseUncertainDelivery,
  summariseUncertainDeliveryOverview
} from './uncertain-delivery';
import type { DashboardJob } from './view-types';

const NOW = Date.parse('2026-08-25T12:00:00.000Z');
const minutesAgo = (count: number) => new Date(NOW - count * 60_000).toISOString();

function job(overrides: Partial<DashboardJob>): DashboardJob {
  return {
    id: 'job_01',
    printerId: 'prt_01',
    agentId: 'agt_01',
    title: 'Order #10838 shipping label',
    source: 'shopify-webhook',
    contentFormat: 'pdf',
    state: 'completed_reported',
    reasonCode: null,
    message: null,
    authority: 'service',
    nativeJobId: null,
    createdAt: minutesAgo(5),
    updatedAt: minutesAgo(5),
    expiresAt: null,
    contentRetained: true,
    ...overrides
  };
}

describe('uncertain delivery summary', () => {
  it('reports nothing to review when no job is uncertain', () => {
    const summary = summariseUncertainDelivery(
      [job({}), job({ state: 'failed_terminal' }), job({ state: 'printing' })],
      NOW
    );

    expect(summary).toEqual({ count: 0, oldestObservedAt: null, oldestLabel: null });
  });

  it('counts uncertain jobs and ages the longest-unresolved one', () => {
    const summary = summariseUncertainDelivery(
      [
        job({ state: 'delivery_uncertain', deliveryUncertainSince: minutesAgo(9) }),
        job({ state: 'delivery_uncertain', deliveryUncertainSince: minutesAgo(125) }),
        job({ state: 'delivery_uncertain', deliveryUncertainSince: minutesAgo(40) }),
        job({ state: 'failed_terminal', updatedAt: minutesAgo(600) })
      ],
      NOW
    );

    expect(summary.count).toBe(3);
    expect(summary.oldestObservedAt).toBe(minutesAgo(125));
    expect(summary.oldestLabel).toBe('2h');
  });

  it('does not let a fresh handoff read as an instant', () => {
    const summary = summariseUncertainDelivery(
      [job({ state: 'delivery_uncertain', deliveryUncertainSince: minutesAgo(0) })],
      NOW
    );

    expect(summary.oldestLabel).toBe('under a minute');
  });

  it('does not infer the transition time from another job timestamp', () => {
    const summary = summariseUncertainDelivery(
      [
        job({
          state: 'delivery_uncertain',
          createdAt: minutesAgo(180),
          updatedAt: minutesAgo(120),
          deliveryUncertainSince: 'not-a-timestamp'
        })
      ],
      NOW
    );

    expect(summary.oldestObservedAt).toBeNull();
    expect(summary.oldestLabel).toBeNull();
  });

  it('still counts a job whose timestamps are unusable', () => {
    const summary = summariseUncertainDelivery(
      [job({ state: 'delivery_uncertain', createdAt: 'nope', updatedAt: 'nope' })],
      NOW
    );

    expect(summary).toEqual({ count: 1, oldestObservedAt: null, oldestLabel: null });
  });

  it('summarises an authoritative paginated count without allocating one item per job', () => {
    expect(summariseUncertainDeliveryOverview(237, minutesAgo(185), NOW)).toEqual({
      count: 237,
      oldestObservedAt: minutesAgo(185),
      oldestLabel: '3h'
    });
  });
});

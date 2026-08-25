import { cleanup, render, screen } from '@testing-library/svelte';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/state', () => ({
  page: { url: new URL('https://piqae.test/dashboard?view=jobs') }
}));

import Page from './+page.svelte';
import type { DashboardJob } from '$lib/view-types';

// Deliberately far from the wall clock: every age below is measured against
// this instant, so a stubbed clock is the only way these assertions can pass.
const NOW = Date.parse('2020-06-01T12:00:00.000Z');
const minutesAgo = (count: number) => new Date(NOW - count * 60_000).toISOString();

function job(overrides: Partial<DashboardJob> = {}): DashboardJob {
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

const agent = {
  id: 'agt_01',
  name: 'Warehouse Mac mini',
  state: 'online',
  os: 'macos',
  architecture: 'arm64',
  version: '0.1.0',
  protocolVersion: '1',
  lastSeenAt: minutesAgo(1),
  queueDepth: 0,
  printerCount: 1,
  labels: []
};

const printer = {
  id: 'prt_01',
  agentId: 'agt_01',
  name: 'Dispatch labels',
  description: null,
  location: null,
  state: 'online',
  stateReasons: [],
  isDefault: true,
  queueDepth: 0,
  lastSeenAt: minutesAgo(1),
  capabilityRevision: 1,
  nativeOptions: {},
  profiles: [],
  capabilities: {
    color: false,
    duplex: false,
    copies: 1,
    papers: [],
    dpis: [],
    source: 'driver',
    revision: '1',
    observedAt: minutesAgo(1)
  }
};

function pageData(jobs: DashboardJob[], stateFilter = 'all') {
  const uncertain = jobs.filter((entry) => entry.state === 'delivery_uncertain').length;
  const failed = jobs.filter((entry) => entry.state.startsWith('failed')).length;
  return {
    dashboardMode: 'live',
    meta: {
      deployment: 'cloud',
      version: '0.1.0',
      auth: { provider: 'workos', workspaceSwitching: false, invitations: false },
      billing: { enabled: false },
      updates: { officialFeed: true, customFeed: false },
      platform: { accounts: false }
    },
    view: 'jobs',
    stateFilter,
    platformEnabled: false,
    overview: {
      agents: { total: 1, online: 1, degraded: 0 },
      printers: { total: 1, online: 1, attention: 0 },
      jobs: { recent: jobs.length, active: 0, failed, uncertain }
    },
    jobs,
    printers: [printer],
    agents: [agent],
    accounts: [],
    detail: null,
    dataError: null
  };
}

const reviewTile = () => screen.getByText('Needs review').closest('a') as HTMLAnchorElement;

describe('uncertain delivery on the operations dashboard', () => {
  beforeAll(() => {
    vi.spyOn(Date, 'now').mockReturnValue(NOW);
    HTMLDialogElement.prototype.showModal = function () {
      this.open = true;
    };
    HTMLDialogElement.prototype.close = function () {
      this.open = false;
    };
  });

  afterEach(cleanup);
  afterAll(() => vi.restoreAllMocks());

  it('stays quiet and goes nowhere special when nothing is uncertain', () => {
    render(Page, {
      data: pageData([job(), job({ id: 'job_02', state: 'printing' })]) as never,
      form: null as never
    });

    const tile = reviewTile();
    expect(tile).toHaveTextContent('No uncertain handoffs');
    expect(tile).not.toHaveClass('attention');
    expect(tile).toHaveAttribute('href', '/dashboard?view=jobs');
    expect(tile.textContent).not.toContain('oldest');
    expect(screen.queryByText('Delivery Uncertain')).not.toBeInTheDocument();
  });

  it('names how long the oldest handoff has been unproven', () => {
    render(Page, {
      data: pageData([
        job({ id: 'job_02', state: 'delivery_uncertain', updatedAt: minutesAgo(125) }),
        job({ id: 'job_03', state: 'delivery_uncertain', updatedAt: minutesAgo(9) })
      ]) as never,
      form: null as never
    });

    const tile = reviewTile();
    expect(tile.textContent).toContain('2 uncertain handoffs');
    expect(tile.textContent).toContain('oldest 2h');
  });

  it('marks the tile for attention and links to exactly those jobs', () => {
    render(Page, {
      data: pageData([
        job({ id: 'job_02', state: 'delivery_uncertain', updatedAt: minutesAgo(185) })
      ]) as never,
      form: null as never
    });

    const tile = reviewTile();
    expect(tile).toHaveClass('attention');
    expect(tile).toHaveAttribute('href', '/dashboard?view=jobs&state=delivery_uncertain');
    expect(tile.textContent).toContain('1 uncertain handoff');
    expect(tile.textContent).toContain('oldest 3h');
    expect(tile.getAttribute('title')).toContain('3h');
  });

  it('does not let a fresh handoff read as resolved', () => {
    render(Page, {
      data: pageData([
        job({ id: 'job_02', state: 'delivery_uncertain', updatedAt: minutesAgo(0) })
      ]) as never,
      form: null as never
    });

    expect(reviewTile().textContent).toContain('oldest under a minute');
  });

  it('shows only uncertain jobs when the address selects that state', () => {
    render(Page, {
      data: pageData(
        [
          job({ id: 'job_01', title: 'Completed slip' }),
          job({ id: 'job_02', title: 'Unproven label', state: 'delivery_uncertain' })
        ],
        'delivery_uncertain'
      ) as never,
      form: null as never
    });

    expect(screen.getByText('Unproven label')).toBeInTheDocument();
    expect(screen.queryByText('Completed slip')).not.toBeInTheDocument();
    expect(screen.getByLabelText('Filter by state')).toHaveValue('delivery_uncertain');
  });
});

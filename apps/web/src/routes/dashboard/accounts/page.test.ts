import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import Page from './+page.svelte';

const accounts = [
  {
    id: 'wsp_01',
    externalId: 'customer:north-star',
    name: 'North Star Coffee',
    status: 'active',
    metadata: {},
    environments: { testId: 'env_test_01', liveId: 'env_live_01' },
    createdAt: '2026-07-28T12:00:00.000Z',
    updatedAt: '2026-07-29T12:00:00.000Z'
  },
  {
    id: 'wsp_02',
    externalId: 'customer:atlas',
    name: 'Atlas Studio',
    status: 'suspended',
    metadata: {},
    environments: { testId: 'env_test_02', liveId: 'env_live_02' },
    createdAt: '2026-07-28T12:00:00.000Z',
    updatedAt: '2026-07-29T12:00:00.000Z'
  }
];

describe('customer accounts dashboard', () => {
  afterEach(cleanup);

  it('shows friendly account data without exposing environment IDs in the list', async () => {
    render(Page, {
      data: { available: true, accounts, dataError: null } as never
    });

    expect(screen.getByRole('heading', { name: 'Customers' })).toBeInTheDocument();
    expect(screen.getByText('North Star Coffee')).toBeInTheDocument();
    expect(screen.getByText('Suspended')).toBeInTheDocument();
    expect(screen.queryByText('env_live_01')).not.toBeInTheDocument();

    await fireEvent.input(screen.getByPlaceholderText('Search customers…'), {
      target: { value: 'atlas' }
    });
    expect(screen.queryByText('North Star Coffee')).not.toBeInTheDocument();
    expect(screen.getByText('Atlas Studio')).toBeInTheDocument();
  });

  it('renders a safe empty state when the deployment capability is disabled', () => {
    render(Page, {
      data: { available: false, accounts: [], dataError: null } as never
    });

    expect(screen.getByText('Customer accounts are not enabled')).toBeInTheDocument();
    expect(screen.queryByPlaceholderText('Search customers…')).not.toBeInTheDocument();
  });
});

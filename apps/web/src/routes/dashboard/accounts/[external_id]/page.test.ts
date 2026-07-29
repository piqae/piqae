import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Page from './+page.svelte';

const account = {
  id: 'wsp_01',
  externalId: 'customer:north-star',
  name: 'North Star Coffee',
  status: 'active',
  metadata: { plan: 'Pro' },
  environments: { testId: 'env_test_01', liveId: 'env_live_01' },
  createdAt: '2026-07-28T12:00:00.000Z',
  updatedAt: '2026-07-29T12:00:00.000Z'
};

describe('customer account detail', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('keeps identifiers under progressive disclosure and provides server-only snippets', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText }
    });
    render(Page, {
      data: { available: true, account, dataError: null } as never
    });

    expect(screen.getByRole('heading', { name: 'North Star Coffee' })).toBeInTheDocument();
    expect(screen.getByText(/trusted backend/)).toBeInTheDocument();
    expect(screen.getByText(/never needed in browser code/)).toBeInTheDocument();
    expect(screen.queryByText(/spl_(live|test)_/)).not.toBeInTheDocument();

    const environmentDetails = screen
      .getByText('Environment IDs')
      .closest('details') as HTMLDetailsElement;
    expect(environmentDetails.open).toBe(false);
    await fireEvent.click(screen.getByText('Environment IDs'));
    expect(environmentDetails.open).toBe(true);

    await fireEvent.click(screen.getByRole('button', { name: 'Copy Live environment ID' }));
    expect(writeText).toHaveBeenCalledWith('env_live_01');
    expect(screen.getByText('Identifier copied')).toBeInTheDocument();
  });

  it('shows a friendly not-found state', () => {
    render(Page, {
      data: { available: true, account: null, dataError: null } as never
    });
    expect(screen.getByRole('heading', { name: 'Customer not found' })).toBeInTheDocument();
  });
});

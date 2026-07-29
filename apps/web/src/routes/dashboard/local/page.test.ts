import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Page from './+page.svelte';

describe('local node dashboard', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('shows native driver profiles, queue truth, and explicit hosted-test confirmation', async () => {
    const fetcher = vi.spyOn(globalThis, 'fetch');
    render(Page, { data: { dashboardMode: 'demo' } as never });

    expect(screen.getByRole('heading', { name: 'Local node' })).toBeInTheDocument();
    expect(screen.getAllByText('Office Laser').length).toBeGreaterThan(0);
    expect(screen.getByText('Dispatch labels')).toBeInTheDocument();
    expect(screen.getAllByText('A4 packing slips').length).toBeGreaterThan(0);
    expect(screen.getAllByText('macOS native · r3').length).toBeGreaterThan(0);
    expect(screen.getByText('stock_a4_plain')).toBeInTheDocument();
    expect(screen.getByText('Available to API')).toBeInTheDocument();
    expect(screen.getByText('copies')).toBeInTheDocument();
    expect(screen.getByText('color')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Validate profile' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Print local test' })).toBeDisabled();
    expect(screen.queryByText('New profile')).not.toBeInTheDocument();
    expect(screen.getByText(/Use the Spool menu bar app/)).toBeInTheDocument();
    expect(screen.getAllByText('Local durable queue').length).toBeGreaterThan(0);
    expect(screen.getByText('macOS / CUPS')).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Hide Office Laser' })).toBeChecked();

    const sendButton = screen
      .getAllByRole('button', { name: 'Send A4 test' })
      .find((button) => !button.hasAttribute('disabled'));
    expect(sendButton).toBeDefined();
    await fireEvent.click(sendButton as HTMLButtonElement);
    expect(screen.getByRole('heading', { name: 'Send A4 test to Office Laser' })).toBeInTheDocument();
    expect(screen.getByText(/hosted durable queue/)).toBeInTheDocument();
    expect(screen.getByText('A4 · Plain paper')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Confirm & send A4 test' })).toBeDisabled();
    expect(fetcher).not.toHaveBeenCalled();
  });
});

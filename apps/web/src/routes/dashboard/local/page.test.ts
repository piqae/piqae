import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Page from './+page.svelte';

describe('local node dashboard', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('shows driver queues, durable profiles, queue truth, and explicit hosted-test confirmation', async () => {
    const fetcher = vi.spyOn(globalThis, 'fetch');
    render(Page, { data: { dashboardMode: 'demo' } as never });

    expect(screen.getByRole('heading', { name: 'Local node' })).toBeInTheDocument();
    expect(screen.getAllByText('Office Laser').length).toBeGreaterThan(0);
    expect(screen.getByText('Dispatch labels')).toBeInTheDocument();
    expect(screen.getByText('A4 packing slips')).toBeInTheDocument();
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
    expect(screen.getByText('A4 · Default media')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Confirm & send A4 test' })).toBeDisabled();
    expect(fetcher).not.toHaveBeenCalled();
  });
});

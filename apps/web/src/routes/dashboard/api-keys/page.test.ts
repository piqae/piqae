import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import Page from './+page.svelte';

const data = {
  apiKeys: [],
  dataError: null,
  dashboardMode: 'live'
};

describe('API-key one-time reveal', () => {
  beforeAll(() => {
    HTMLDialogElement.prototype.showModal = function () {
      this.open = true;
    };
    HTMLDialogElement.prototype.close = function () {
      this.open = false;
      this.dispatchEvent(new Event('close'));
    };
  });

  afterEach(cleanup);

  it('shows plaintext only in the immediate create action result', async () => {
    const secret = 'spl_live_once_only';
    render(Page, {
      data: data as never,
      form: {
        mutation: 'createApiKey',
        apiKey: {
          id: 'key_01',
          name: 'Order service',
          prefix: 'spl_live_abcd',
          secret
        }
      } as never
    });

    expect(screen.getByText('Secret key · shown once')).toBeInTheDocument();
    expect(screen.getByText(secret)).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Create secret key' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.queryByText(secret)).not.toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Create secret key' }));
    expect(screen.queryByText(secret)).not.toBeInTheDocument();

    cleanup();
    render(Page, { data: data as never, form: null });
    expect(screen.queryByText(secret)).not.toBeInTheDocument();
  });
});

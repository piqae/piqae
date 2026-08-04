import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import Page from './+page.svelte';

const meta = {
  deployment: 'cloud',
  version: '0.1.0',
  auth: { provider: 'workos', workspaceSwitching: false, invitations: false },
  billing: { enabled: false },
  updates: { officialFeed: true, customFeed: false },
  platform: { accounts: false }
};

const data = {
  dashboardMode: 'live',
  meta,
  viewer: null,
  sections: { team: false, billing: false },
  billingContext: {
    available: false,
    canManageBilling: false,
    pricing: { version: 'test', plans: [{ plan: 'pro' }] },
    selectedInterval: 'monthly',
    checkoutState: null,
    checkoutAvailable: { monthly: false, annual: false },
    portalAvailable: false
  },
  apiKeys: Promise.resolve({ items: [], dataError: null }),
  webhooks: Promise.resolve({ items: [], dataError: null }),
  team: null,
  billing: null
};

describe('settings credential reveal', () => {
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

  it('shows an API key secret only in the immediate create action result', async () => {
    const secret = 'piq_live_once_only';
    render(Page, {
      data: data as never,
      form: {
        mutation: 'createApiKey',
        apiKey: {
          id: 'key_01',
          name: 'Order service',
          prefix: 'piq_live_abcd',
          secret
        }
      } as never
    });

    await fireEvent.click(screen.getByRole('button', { name: 'Create secret key' }));
    expect(screen.getByText('Secret key · shown once')).toBeInTheDocument();
    expect(screen.getByText(secret)).toBeInTheDocument();

    // Dismissing the dialog ends the reveal session for good.
    await fireEvent.click(screen.getByRole('button', { name: 'Close secret key dialog' }));
    expect(screen.queryByText(secret)).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Create secret key' }));
    expect(screen.queryByText(secret)).not.toBeInTheDocument();

    cleanup();
    render(Page, { data: data as never, form: null });
    expect(screen.queryByText(secret)).not.toBeInTheDocument();
  });

  it('shows a webhook signing secret only in the immediate create action result', async () => {
    const secret = 'whsec_once_only';
    render(Page, {
      data: data as never,
      form: {
        mutation: 'createWebhook',
        webhook: { id: 'whk_01', url: 'https://example.test/piqae', secret }
      } as never
    });

    await fireEvent.click(screen.getByRole('button', { name: 'Add endpoint' }));
    expect(screen.getByText('Signing secret · shown once')).toBeInTheDocument();
    expect(screen.getByText(secret)).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Close webhook dialog' }));
    expect(screen.queryByText(secret)).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Add endpoint' }));
    expect(screen.queryByText(secret)).not.toBeInTheDocument();
  });

  it('disables every mutating control while demo data is active', async () => {
    render(Page, {
      data: { ...data, dashboardMode: 'demo' } as never,
      form: null
    });

    await fireEvent.click(screen.getByRole('button', { name: 'Create secret key' }));
    const dialog = screen.getByRole('dialog', { name: 'Create secret key' });
    expect(dialog.querySelector('button[type="submit"]')).toBeDisabled();
    expect(screen.getByText('Demo mode: preview only. No credential will be created.')).toBeInTheDocument();
  });
});

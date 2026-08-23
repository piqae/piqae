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
  workspace: Promise.resolve({
    workspace: { id: 'wsp_test', name: 'Test workspace', slug: 'test-workspace' },
    dataError: null
  }),
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

  it('lists the platform credential with rotate and revoke actions', async () => {
    render(Page, {
      data: {
        ...data,
        meta: { ...meta, platform: { accounts: true } },
        sections: { team: false, billing: false, platform: true },
        platform: Promise.resolve({ enabled: true, dataError: null }),
        apiKeys: Promise.resolve({
          items: [
            {
              id: '00000000-0000-4000-8000-000000000001',
              name: 'Piqae platform integration',
              prefix: 'piq_platform_00000000',
              environment: 'platform',
              kind: 'platform',
              scopes: [],
              lastUsedAt: null,
              createdAt: '2026-08-20T00:00:00Z'
            }
          ],
          dataError: null
        })
      } as never,
      form: null
    });

    expect(await screen.findByText('Piqae platform integration')).toBeInTheDocument();
    expect(screen.getByText('Customer accounts')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Rotate' })).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Revoke Piqae platform integration' })
    ).toBeInTheDocument();
  });
});

describe('workspace rename', () => {
  it('shows the current name and warns when the directory mirror fails', async () => {
    render(Page, {
      data,
      form: {
        mutation: 'renameWorkspace',
        workspace: { id: 'wsp_test', name: 'Renamed workspace', slug: 'test-workspace' },
        directoryWarning: 'The workspace was renamed, but the linked WorkOS organisation still shows the old name.'
      }
    });

    // The rename result wins over the loaded value so the field never snaps
    // back to the stale name after a successful save.
    expect(await screen.findByDisplayValue('Renamed workspace')).toBeTruthy();
    expect(screen.getByText(/still shows the old name/)).toBeTruthy();
  });
});


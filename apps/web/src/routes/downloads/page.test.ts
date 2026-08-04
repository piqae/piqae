import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { loadReleaseManifest } from '$lib/server/release-manifest';

vi.mock('$app/navigation', async (importOriginal) => ({
  ...(await importOriginal<typeof import('$app/navigation')>()),
  replaceState: (url: string | URL) => window.history.replaceState({}, '', url)
}));

import Page from './+page.svelte';

describe('downloads', () => {
  afterEach(cleanup);

  it('removes a connect capability from the URL before offering an app handoff', async () => {
    const token = `piq_enr_${'a'.repeat(32)}`;
    window.history.replaceState({}, '', `/downloads#enrolment_token=${token}`);

    render(Page, {
      data: {
        meta: {
          deployment: 'cloud',
          version: '0.1.0',
          auth: { provider: 'workos', workspaceSwitching: true, invitations: true },
          billing: { enabled: true },
          updates: { officialFeed: true, customFeed: false }
        },
        manifest: loadReleaseManifest({}),
        detected: { platform: 'macos', architecture: null, label: 'this Mac' },
        recommendedArtifactId: 'macos-universal'
      } as never
    });

    expect(
      await screen.findByRole('heading', { name: 'Connect the Piqae app on this computer.' })
    ).toBeInTheDocument();
    expect(window.location.hash).toBe('');
    expect(document.body).not.toHaveTextContent(token);
    expect(screen.getByRole('button', { name: 'Open Piqae to connect' })).toBeInTheDocument();
    expect(screen.getByText(/do not need a separate Piqae account/i)).toBeInTheDocument();
  });

  it('leads with the detected platform while keeping unsupported builds truthful', () => {
    render(Page, {
      data: {
        meta: {
          deployment: 'cloud',
          version: '0.1.0',
          auth: { provider: 'workos', workspaceSwitching: true, invitations: true },
          billing: { enabled: true },
          updates: { officialFeed: true, customFeed: false }
        },
        manifest: loadReleaseManifest({}),
        detected: { platform: 'macos', architecture: null, label: 'this Mac' },
        recommendedArtifactId: 'macos-universal'
      } as never
    });

    expect(
      screen.getByRole('heading', { name: 'Piqae for macOS is almost ready' })
    ).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /View Mac release status/ })).toHaveAttribute(
      'href',
      'https://github.com/piqae/piqae/releases'
    );
    expect(screen.getByText('This device')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Ready to print in minutes.' })).toBeInTheDocument();
    expect(screen.getByText('Connect your account')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Connect a computer/ })).toHaveAttribute('href', '/pair');
    expect(
      screen.getByRole('heading', { name: 'Everything technical, when you need it.' })
    ).toBeInTheDocument();
    expect(screen.getAllByText('macOS')).not.toHaveLength(0);
    expect(screen.queryByRole('link', { name: /^Download / })).not.toBeInTheDocument();
  });

  it.each([
    {
      status: 'preview' as const,
      heading: 'Download the Piqae preview for macOS',
      link: 'Download preview for Mac'
    },
    {
      status: 'supported' as const,
      heading: 'Download Piqae for macOS',
      link: 'Download for Mac'
    }
  ])('shows a real primary download for a signed $status artifact', ({ status, heading, link }) => {
    const manifest = loadReleaseManifest({});
    const mac = manifest.artifacts.find((artifact) => artifact.id === 'macos-universal');
    if (!mac) throw new Error('macOS fixture missing');
    mac.status = status;
    mac.fileName = 'Piqae.dmg';
    mac.downloadUrl = 'https://github.com/piqae/piqae/releases/download/v0.1.0/Piqae.dmg';
    mac.sha256 = 'a'.repeat(64);
    mac.signing = { status: 'verified', label: 'Signed and notarised by Apple' };

    render(Page, {
      data: {
        meta: {
          deployment: 'cloud',
          version: '0.1.0',
          auth: { provider: 'workos', workspaceSwitching: true, invitations: true },
          billing: { enabled: true },
          updates: { officialFeed: true, customFeed: false }
        },
        manifest,
        detected: { platform: 'macos', architecture: null, label: 'this Mac' },
        recommendedArtifactId: 'macos-universal'
      } as never
    });

    expect(screen.getByRole('heading', { name: heading })).toBeInTheDocument();
    const downloadLinks = screen.getAllByRole('link', { name: link });
    expect(downloadLinks).toHaveLength(2);
    for (const downloadLink of downloadLinks) {
      expect(downloadLink).toHaveAttribute('href', mac.downloadUrl);
    }
  });
});

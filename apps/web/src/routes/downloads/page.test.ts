import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { loadReleaseManifest } from '$lib/server/release-manifest';
import Page from './+page.svelte';

describe('downloads', () => {
  afterEach(cleanup);

  it('renders server-owned evidence and browser-pairing onboarding', () => {
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

    expect(screen.getByRole('heading', { name: 'Connect a printer computer' })).toBeInTheDocument();
    expect(screen.getByText('Detected this Mac')).toBeInTheDocument();
    expect(screen.getByText('Development only')).toBeInTheDocument();
    expect(screen.getAllByText('Preview')).toHaveLength(2);
    expect(screen.getAllByText('Not published')).toHaveLength(3);
    expect(screen.getByText('Unsigned Preview build')).toBeInTheDocument();
    expect(
      screen.getByRole('heading', { name: 'Approve the computer in your browser' })
    ).toBeInTheDocument();
    expect(screen.getByText('Connect node')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Open pairing/ })).toHaveAttribute('href', '/pair');
    expect(screen.getByRole('heading', { name: 'Older releases' })).toBeInTheDocument();
    expect(screen.getByText(/No older server-owned releases/)).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: /^Download / })).not.toBeInTheDocument();
  });
});

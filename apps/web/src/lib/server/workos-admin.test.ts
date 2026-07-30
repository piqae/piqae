import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('$lib/server/auth-config', () => ({
  workosConfig: { apiKey: 'sk_test_redacted' }
}));

import {
  WorkOsAdminError,
  listOrganizationInvitations,
  listUserMemberships,
  organizationRoles
} from './workos-admin';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('WorkOS administration boundary', () => {
  it('keeps the complete production role set', () => {
    expect(organizationRoles()).toEqual([
      'owner',
      'admin',
      'developer',
      'operator',
      'viewer',
      'billing'
    ]);
  });

  it('returns organization-scoped membership data', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        Response.json({
          data: [
            {
              id: 'om_test',
              organization_id: 'org_test',
              organization_name: 'Test workspace',
              user_id: 'user_test',
              status: 'active',
              role: { slug: 'operator' }
            }
          ]
        })
      )
    );
    await expect(listUserMemberships('user_test')).resolves.toEqual([
      {
        id: 'om_test',
        organizationId: 'org_test',
        organizationName: 'Test workspace',
        userId: 'user_test',
        status: 'active',
        role: 'operator'
      }
    ]);
  });

  it('never returns invitation tokens or acceptance URLs', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        Response.json({
          data: [
            {
              id: 'invitation_test',
              email: 'invitee@example.test',
              state: 'pending',
              organization_id: 'org_test',
              role_slug: 'viewer',
              expires_at: '2026-08-13T00:00:00Z',
              created_at: '2026-07-30T00:00:00Z',
              token: 'must-not-escape',
              accept_invitation_url: 'https://example.test/must-not-escape'
            }
          ]
        })
      )
    );
    const invitations = await listOrganizationInvitations('org_test');
    expect(invitations).toEqual([
      {
        id: 'invitation_test',
        email: 'invitee@example.test',
        state: 'pending',
        role: 'viewer',
        expiresAt: '2026-08-13T00:00:00Z',
        createdAt: '2026-07-30T00:00:00Z'
      }
    ]);
    expect(JSON.stringify(invitations)).not.toContain('must-not-escape');
  });

  it('sanitizes upstream error bodies', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        Response.json(
          { code: 'invalid_request', message: 'sensitive upstream detail' },
          { status: 422 }
        )
      )
    );
    const request = listUserMemberships('user_test');
    await expect(request).rejects.toBeInstanceOf(WorkOsAdminError);
    await expect(request).rejects.not.toThrow('sensitive upstream detail');
  });
});

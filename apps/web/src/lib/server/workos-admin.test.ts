import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('$lib/server/auth-config', () => ({
  workosConfig: { apiKey: 'sk_test_redacted' }
}));

import {
  WorkOsAdminError,
  createOrganization,
  ensureOrganizationMembership,
  listOrganizationInvitations,
  listOrganizationMembers,
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
              role: { slug: 'operator' },
              user: {
                id: 'user_test',
                email: 'operator@example.test',
                first_name: null,
                last_name: null
              }
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

  it('aggregates every membership page using embedded user data', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        Response.json({
          data: [
            {
              id: 'om_first',
              organization_id: 'org_test',
              organization_name: 'Test workspace',
              user_id: 'user_first',
              status: 'active',
              role: { slug: 'admin' },
              user: {
                id: 'user_first',
                email: 'first@example.test',
                first_name: 'First',
                last_name: 'Member'
              }
            }
          ],
          list_metadata: { after: 'cursor_next' }
        })
      )
      .mockResolvedValueOnce(
        Response.json({
          data: [
            {
              id: 'om_second',
              organization_id: 'org_test',
              organization_name: 'Test workspace',
              user_id: 'user_second',
              status: 'active',
              role: { slug: 'viewer' },
              user: {
                id: 'user_second',
                email: 'second@example.test',
                first_name: null,
                last_name: null
              }
            }
          ],
          list_metadata: { after: null }
        })
      );
    vi.stubGlobal('fetch', fetchMock);

    const members = await listOrganizationMembers('org_test');

    expect(members.map((member) => member.id)).toEqual(['om_first', 'om_second']);
    expect(members[0]?.name).toBe('First Member');
    expect(members[1]?.email).toBe('second@example.test');
    expect(fetchMock.mock.calls[1]?.[0]).toContain('after=cursor_next');
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

  it('persists and reuses the onboarding recovery key in WorkOS', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(Response.json({ code: 'not_found' }, { status: 404 }))
      .mockResolvedValueOnce(Response.json({ id: 'org_test', name: 'Test workspace' }))
      .mockResolvedValueOnce(Response.json({ id: 'org_test', name: 'Test workspace' }));
    vi.stubGlobal('fetch', fetchMock);

    await expect(createOrganization('Test workspace', 'piqae:user:token')).resolves.toEqual({
      id: 'org_test',
      name: 'Test workspace'
    });
    await expect(createOrganization('Test workspace', 'piqae:user:token')).resolves.toEqual({
      id: 'org_test',
      name: 'Test workspace'
    });

    const createRequest = fetchMock.mock.calls[1] as [string, RequestInit];
    expect(JSON.parse(String(createRequest[1].body))).toEqual({
      name: 'Test workspace',
      external_id: 'piqae:user:token'
    });
    expect((createRequest[1].headers as Headers).get('Idempotency-Key')).toBe(
      'piqae:user:token'
    );
    expect(fetchMock.mock.calls[2]?.[0]).toContain(
      '/organizations/external_id/piqae%3Auser%3Atoken'
    );
  });

  it('makes owner membership creation retry-safe', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(Response.json({ data: [] }))
      .mockResolvedValueOnce(Response.json({ id: 'om_test' }));
    vi.stubGlobal('fetch', fetchMock);

    await ensureOrganizationMembership(
      'org_test',
      'user_test',
      'owner',
      'piqae:user:token:owner-membership'
    );

    const createRequest = fetchMock.mock.calls[1] as [string, RequestInit];
    expect((createRequest[1].headers as Headers).get('Idempotency-Key')).toBe(
      'piqae:user:token:owner-membership'
    );
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

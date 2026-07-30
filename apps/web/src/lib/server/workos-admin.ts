import { workosConfig } from '$lib/server/auth-config';

const WORKOS_API_ORIGIN = 'https://api.workos.com';
const REQUEST_TIMEOUT_MS = 8_000;
const ORGANIZATION_ROLES = [
  'owner',
  'admin',
  'developer',
  'operator',
  'viewer',
  'billing'
] as const;

export type OrganizationRole = (typeof ORGANIZATION_ROLES)[number];

interface WorkOsList<T> {
  data: T[];
}

interface WorkOsOrganization {
  id: string;
  name: string;
}

interface WorkOsRole {
  slug: string;
}

interface WorkOsMembership {
  id: string;
  organization_id: string;
  organization_name: string;
  user_id: string;
  status: 'active' | 'inactive' | 'pending';
  role: WorkOsRole;
  roles?: WorkOsRole[];
}

interface WorkOsUser {
  id: string;
  email: string;
  first_name: string | null;
  last_name: string | null;
}

interface WorkOsInvitation {
  id: string;
  email: string;
  state: 'pending' | 'accepted' | 'expired' | 'revoked';
  organization_id: string | null;
  role_slug: string | null;
  expires_at: string;
  created_at: string;
}

export interface SafeOrganizationMembership {
  id: string;
  organizationId: string;
  organizationName: string;
  userId: string;
  status: 'active' | 'inactive' | 'pending';
  role: OrganizationRole;
}

export interface SafeOrganizationMember extends SafeOrganizationMembership {
  email: string;
  name: string | null;
}

export interface SafeInvitation {
  id: string;
  email: string;
  state: 'pending' | 'accepted' | 'expired' | 'revoked';
  role: OrganizationRole;
  expiresAt: string;
  createdAt: string;
}

export class WorkOsAdminError extends Error {
  constructor(
    readonly status: number,
    readonly kind: string
  ) {
    super('WorkOS administration request failed');
    this.name = 'WorkOsAdminError';
  }
}

export function isOrganizationRole(value: string): value is OrganizationRole {
  return ORGANIZATION_ROLES.includes(value as OrganizationRole);
}

export function organizationRoles(): readonly OrganizationRole[] {
  return ORGANIZATION_ROLES;
}

export async function listUserMemberships(userId: string): Promise<SafeOrganizationMembership[]> {
  const response = await workosRequest<WorkOsList<WorkOsMembership>>(
    `/user_management/organization_memberships?user_id=${encodeURIComponent(userId)}&statuses=active&limit=100`
  );
  return response.data.map(safeMembership).filter((value) => value !== null);
}

export async function listOrganizationMembers(
  organizationId: string
): Promise<SafeOrganizationMember[]> {
  const [memberships, users] = await Promise.all([
    workosRequest<WorkOsList<WorkOsMembership>>(
      `/user_management/organization_memberships?organization_id=${encodeURIComponent(organizationId)}&limit=100`
    ),
    workosRequest<WorkOsList<WorkOsUser>>(
      `/user_management/users?organization_id=${encodeURIComponent(organizationId)}&limit=100`
    )
  ]);
  const usersById = new Map(users.data.map((user) => [user.id, user]));
  return memberships.data.flatMap((membership) => {
    const safe = safeMembership(membership);
    const user = usersById.get(membership.user_id);
    if (!safe || !user) return [];
    const name = [user.first_name, user.last_name].filter(Boolean).join(' ');
    return [
      {
        ...safe,
        email: user.email,
        name: name || null
      }
    ];
  });
}

export async function listOrganizationInvitations(
  organizationId: string
): Promise<SafeInvitation[]> {
  const response = await workosRequest<WorkOsList<WorkOsInvitation>>(
    `/user_management/invitations?organization_id=${encodeURIComponent(organizationId)}&limit=100`
  );
  return response.data.flatMap((invitation) => {
    if (!invitation.role_slug || !isOrganizationRole(invitation.role_slug)) return [];
    return [
      {
        id: invitation.id,
        email: invitation.email,
        state: invitation.state,
        role: invitation.role_slug,
        expiresAt: invitation.expires_at,
        createdAt: invitation.created_at
      }
    ];
  });
}

export async function createOrganization(
  name: string,
  idempotencyKey: string
): Promise<WorkOsOrganization> {
  return workosRequest('/organizations', {
    method: 'POST',
    body: { name },
    idempotencyKey
  });
}

export async function ensureOrganizationMembership(
  organizationId: string,
  userId: string,
  role: OrganizationRole
): Promise<void> {
  const memberships = await listUserMemberships(userId);
  if (memberships.some((membership) => membership.organizationId === organizationId)) return;
  await workosRequest('/user_management/organization_memberships', {
    method: 'POST',
    body: {
      organization_id: organizationId,
      user_id: userId,
      role_slug: role
    }
  });
}

export async function sendInvitation(
  organizationId: string,
  inviterUserId: string,
  email: string,
  role: OrganizationRole
): Promise<void> {
  await workosRequest('/user_management/invitations', {
    method: 'POST',
    body: {
      email,
      organization_id: organizationId,
      inviter_user_id: inviterUserId,
      role_slug: role,
      expires_in_days: 14
    }
  });
}

export async function updateMembershipRole(
  membershipId: string,
  role: OrganizationRole
): Promise<void> {
  await workosRequest(`/user_management/organization_memberships/${encodeURIComponent(membershipId)}`, {
    method: 'PUT',
    body: { role_slug: role }
  });
}

export async function removeMembershipAccess(membershipId: string): Promise<void> {
  await workosRequest(
    `/user_management/organization_memberships/${encodeURIComponent(membershipId)}/deactivate`,
    { method: 'PUT', body: {} }
  );
}

export async function revokeInvitation(invitationId: string): Promise<void> {
  await workosRequest(`/user_management/invitations/${encodeURIComponent(invitationId)}/revoke`, {
    method: 'POST'
  });
}

export async function resendInvitation(invitationId: string): Promise<void> {
  await workosRequest(`/user_management/invitations/${encodeURIComponent(invitationId)}/resend`, {
    method: 'POST',
    body: {}
  });
}

function safeMembership(membership: WorkOsMembership): SafeOrganizationMembership | null {
  const role = membership.role?.slug ?? membership.roles?.[0]?.slug;
  if (!role || !isOrganizationRole(role)) return null;
  return {
    id: membership.id,
    organizationId: membership.organization_id,
    organizationName: membership.organization_name,
    userId: membership.user_id,
    status: membership.status,
    role
  };
}

async function workosRequest<T>(
  path: string,
  options: {
    method?: 'GET' | 'POST' | 'PUT';
    body?: Record<string, unknown>;
    idempotencyKey?: string;
  } = {}
): Promise<T> {
  const apiKey = workosConfig?.apiKey;
  if (!apiKey) throw new WorkOsAdminError(503, 'not_configured');
  const headers = new Headers({
    Authorization: `Bearer ${apiKey}`,
    'Content-Type': 'application/json'
  });
  if (options.idempotencyKey) headers.set('Idempotency-Key', options.idempotencyKey);
  let response: Response;
  try {
    response = await fetch(`${WORKOS_API_ORIGIN}${path}`, {
      method: options.method ?? 'GET',
      headers,
      body: options.body === undefined ? undefined : JSON.stringify(options.body),
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS)
    });
  } catch {
    throw new WorkOsAdminError(503, 'unavailable');
  }
  if (!response.ok) {
    let kind = `http_${response.status}`;
    try {
      const error = (await response.json()) as { code?: unknown; error?: unknown };
      if (typeof error.code === 'string') kind = error.code;
      else if (typeof error.error === 'string') kind = error.error;
    } catch {
      // Response details are intentionally discarded so secrets and identity data cannot escape.
    }
    throw new WorkOsAdminError(response.status, kind);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

import { fail, redirect, type RequestEvent } from '@sveltejs/kit';
import type { ApiKeyScope } from '@piqae/sdk';
import type { Actions, PageServerLoad } from './$types';
import type { MarketingAttribution } from '$lib/marketing/attribution';
import { authMode } from '$lib/server/auth-config';
import {
  canManageHostedBilling,
  checkoutAllowed,
  parseBillingSummary,
  parseUsageSummary,
  stripePortalAvailable
} from '$lib/server/billing';
import {
  dashboardConnection,
  dashboardMode,
  dashboardSdk,
  dashboardSource,
  preventSecretCaching,
  presentDashboardError
} from '$lib/server/dashboard-data';
import { pricingCatalog } from '$lib/server/pricing';
import {
  WorkOsAdminError,
  isOrganizationRole,
  listOrganizationInvitations,
  listOrganizationMembers,
  listUserMemberships,
  organizationRoles,
  removeMembershipAccess,
  resendInvitation as resendWorkosInvitation,
  revokeInvitation as revokeWorkosInvitation,
  sendInvitation,
  updateMembershipRole,
  type OrganizationRole,
  type SafeOrganizationMember
} from '$lib/server/workos-admin';

const allowedScopes = new Set<ApiKeyScope>([
  'api_keys_read',
  'api_keys_write',
  'agents_read',
  'agents_write',
  'printers_read',
  'printers_write',
  'jobs_read',
  'jobs_write',
  'webhooks_read',
  'webhooks_write',
  'usage_read',
  'audit_read'
]);

/**
 * Every configuration surface lives on this one route. Only the flags that
 * decide which sections render are awaited; the sections themselves stream in
 * so opening Settings never waits on WorkOS, Stripe and the control plane at
 * once. Streamed helpers must resolve rather than reject — a rejected promise
 * would fail the whole page.
 */
export const load: PageServerLoad = async (event) => {
  preventSecretCaching(event);
  const { meta } = await event.parent();
  const canManageBilling = canManageHostedBilling(event.locals.auth?.role);

  return {
    sections: {
      platform: meta.platform.accounts,
      team: meta.auth.invitations && authMode === 'workos',
      billing: meta.billing.enabled
    },
    billingContext: {
      available: meta.billing.enabled,
      canManageBilling,
      pricing: pricingCatalog(),
      selectedInterval: preferredInterval(event),
      checkoutState: event.url.searchParams.get('checkout'),
      checkoutAvailable: {
        monthly: checkoutAllowed('pro', 'monthly'),
        annual: checkoutAllowed('pro', 'annual')
      },
      portalAvailable: stripePortalAvailable()
    },
    apiKeys: loadApiKeys(event),
    platform: meta.platform.accounts ? loadPlatform(event) : null,
    webhooks: loadWebhooks(event),
    team: meta.auth.invitations && authMode === 'workos' ? loadTeam(event) : null,
    billing: meta.billing.enabled ? loadBilling(event) : null
  };
};

async function loadPlatform(event: RequestEvent) {
  try {
    return {
      enabled: await dashboardSource(event).api.platformEnabled(),
      dataError: null
    };
  } catch (error) {
    return { enabled: false, dataError: presentDashboardError(error) };
  }
}

function preferredInterval(event: RequestEvent): 'monthly' | 'annual' {
  const stored = event.cookies.get('piqae_attribution');
  if (!stored) return 'monthly';
  try {
    const attribution = JSON.parse(
      Buffer.from(stored, 'base64url').toString('utf8')
    ) as MarketingAttribution;
    return attribution?.interval === 'annual' ? 'annual' : 'monthly';
  } catch {
    return 'monthly';
  }
}

async function loadApiKeys(event: RequestEvent) {
  try {
    const apiKeys = await dashboardSource(event).api.apiKeys();
    return { items: apiKeys.data, dataError: null };
  } catch (error) {
    return { items: [], dataError: presentDashboardError(error) };
  }
}

async function loadWebhooks(event: RequestEvent) {
  try {
    const webhooks = await dashboardSource(event).api.webhooks();
    return { items: webhooks.data, dataError: null };
  } catch (error) {
    return { items: [], dataError: presentDashboardError(error) };
  }
}

async function loadTeam(event: RequestEvent) {
  const current = actor(event);
  if (!current) return null;
  try {
    const [members, invitations, workspaces] = await Promise.all([
      listOrganizationMembers(current.organizationId),
      listOrganizationInvitations(current.organizationId),
      listUserMemberships(current.userId)
    ]);
    return {
      members,
      invitations: invitations.filter((invitation) => invitation.state === 'pending'),
      workspaces,
      roles: organizationRoles(),
      canManage: canManage(current.role),
      dataError: null
    };
  } catch (error) {
    return {
      members: [],
      invitations: [],
      workspaces: [],
      roles: organizationRoles(),
      canManage: false,
      dataError: presentDashboardError(error)
    };
  }
}

async function loadBilling(event: RequestEvent) {
  try {
    const { baseUrl, bearerToken } = dashboardConnection(event);
    const headers = {
      accept: 'application/json',
      authorization: `Bearer ${bearerToken}`,
      'x-piqae-dashboard': '1'
    };
    const [summaryResponse, usageResponse] = await Promise.all([
      event.fetch(`${baseUrl.replace(/\/$/, '')}/v1/billing/summary`, { headers }),
      event.fetch(`${baseUrl.replace(/\/$/, '')}/v1/usage`, { headers })
    ]);
    if (!summaryResponse.ok || !usageResponse.ok) {
      throw new Error(
        `Piqae billing request failed with HTTP ${summaryResponse.status}/${usageResponse.status}.`
      );
    }
    return {
      summary: parseBillingSummary(await summaryResponse.json()),
      workspaceUsage: parseUsageSummary(await usageResponse.json()),
      dataError: null
    };
  } catch (error) {
    return { summary: null, workspaceUsage: null, dataError: presentDashboardError(error) };
  }
}

export const actions: Actions = {
  enablePlatform: async (event) => {
    preventSecretCaching(event);
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'enablePlatform',
        error: { message: 'Platform-mode mutations are disabled while demo data is active.' }
      });
    }
    const current = actor(event);
    if (authMode === 'workos' && (!current || !canManage(current.role))) {
      return fail(403, {
        mutation: 'enablePlatform',
        error: { message: 'Only workspace owners and admins can enable platform mode.' }
      });
    }
    try {
      const result = await dashboardSource(event).api.enablePlatform();
      return {
        mutation: 'enablePlatform',
        platform: result
      };
    } catch (error) {
      return fail(502, {
        mutation: 'enablePlatform',
        error: { message: presentDashboardError(error).message }
      });
    }
  },

  createApiKey: async (event) => {
    preventSecretCaching(event);
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'createApiKey',
        error: { message: 'API-key mutations are disabled while demo data is active.' }
      });
    }

    const data = await event.request.formData();
    const name = String(data.get('name') ?? '').trim();
    const requestedScopes = [...new Set(data.getAll('scopes').map(String))];
    const expiresAtValue = String(data.get('expires_at') ?? '').trim();
    if (name.length < 2 || name.length > 120) {
      return fail(400, {
        mutation: 'createApiKey',
        error: { message: 'Key name must be between 2 and 120 characters.' }
      });
    }
    if (
      requestedScopes.length === 0 ||
      requestedScopes.some((scope) => !allowedScopes.has(scope as ApiKeyScope))
    ) {
      return fail(400, {
        mutation: 'createApiKey',
        error: { message: 'Select one or more supported API scopes.' }
      });
    }

    let expiresAt: string | null = null;
    if (expiresAtValue) {
      const parsed = new Date(expiresAtValue);
      if (Number.isNaN(parsed.valueOf()) || parsed <= new Date()) {
        return fail(400, {
          mutation: 'createApiKey',
          error: { message: 'Expiry must be a valid future date.' }
        });
      }
      expiresAt = parsed.toISOString();
    }

    try {
      const apiKey = await dashboardSdk(event).apiKeys.create({
        name,
        scopes: requestedScopes as ApiKeyScope[],
        expires_at: expiresAt
      });
      return {
        mutation: 'createApiKey',
        apiKey: {
          id: apiKey.id,
          name: apiKey.name,
          prefix: apiKey.lookup_prefix,
          secret: apiKey.secret
        }
      };
    } catch (error) {
      return fail(502, {
        mutation: 'createApiKey',
        error: { message: presentDashboardError(error).message }
      });
    }
  },

  revokeApiKey: async (event) => {
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'revokeApiKey',
        error: { message: 'API-key mutations are disabled while demo data is active.' }
      });
    }
    const data = await event.request.formData();
    const apiKeyId = String(data.get('api_key_id') ?? '').trim();
    if (!apiKeyId) {
      return fail(400, {
        mutation: 'revokeApiKey',
        error: { message: 'API key ID is required.' }
      });
    }
    try {
      await dashboardSdk(event).apiKeys.revoke(apiKeyId);
      return { mutation: 'revokeApiKey', revokedApiKeyId: apiKeyId };
    } catch (error) {
      return fail(502, {
        mutation: 'revokeApiKey',
        error: { message: presentDashboardError(error).message }
      });
    }
  },

  createWebhook: async (event) => {
    preventSecretCaching(event);
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'createWebhook',
        error: { message: 'Webhook mutations are disabled while demo data is active.' }
      });
    }
    const data = await event.request.formData();
    const url = String(data.get('url') ?? '').trim();
    const events = data.getAll('events').map(String).filter(Boolean);
    try {
      const parsed = new URL(url);
      if (!['https:', 'http:'].includes(parsed.protocol)) throw new Error('invalid protocol');
    } catch {
      return fail(400, {
        mutation: 'createWebhook',
        error: { message: 'Enter a valid HTTP or HTTPS webhook URL.' }
      });
    }
    if (events.length === 0) {
      return fail(400, {
        mutation: 'createWebhook',
        error: { message: 'Select at least one event family.' }
      });
    }
    try {
      const webhook = await dashboardSdk(event).webhooks.create({ url, events });
      return {
        mutation: 'createWebhook',
        webhook: { id: webhook.id, url: webhook.url, secret: webhook.secret }
      };
    } catch (error) {
      return fail(502, {
        mutation: 'createWebhook',
        error: { message: presentDashboardError(error).message }
      });
    }
  },

  deleteWebhook: async (event) => {
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'deleteWebhook',
        error: { message: 'Webhook mutations are disabled while demo data is active.' }
      });
    }
    const data = await event.request.formData();
    const webhookId = String(data.get('webhook_id') ?? '').trim();
    if (!webhookId) {
      return fail(400, {
        mutation: 'deleteWebhook',
        error: { message: 'Webhook ID is required.' }
      });
    }
    try {
      await dashboardSdk(event).webhooks.remove(webhookId);
      return { mutation: 'deleteWebhook', deletedWebhookId: webhookId };
    } catch (error) {
      return fail(502, {
        mutation: 'deleteWebhook',
        error: { message: presentDashboardError(error).message }
      });
    }
  },

  inviteMember: async (event) => {
    const current = workosActor(event);
    if (!canManage(current.role)) {
      return actionFailure(403, 'Only owners and admins can invite members.');
    }
    const form = await event.request.formData();
    const email = text(form, 'email').toLowerCase();
    const role = text(form, 'role');
    if (!validEmail(email) || !isOrganizationRole(role)) {
      return actionFailure(400, 'Enter a valid email address and role.');
    }
    if (role === 'owner' && current.role !== 'owner') {
      return actionFailure(403, 'Only an owner can invite another owner.');
    }
    try {
      await sendInvitation(current.organizationId, current.userId, email, role);
      return { error: false, message: `Invitation sent to ${email}.` };
    } catch (error) {
      return workosFailure(error, 'The invitation could not be sent.');
    }
  },

  updateMemberRole: async (event) => {
    const current = workosActor(event);
    if (!canManage(current.role)) {
      return actionFailure(403, 'Only owners and admins can update roles.');
    }
    const form = await event.request.formData();
    const membershipId = text(form, 'membership_id');
    const role = text(form, 'role');
    if (!membershipId || !isOrganizationRole(role)) {
      return actionFailure(400, 'Choose a valid member and role.');
    }
    const members = await listOrganizationMembers(current.organizationId);
    const target = members.find((member) => member.id === membershipId);
    if (!target) return actionFailure(404, 'That member is not in this workspace.');
    const authorizationFailure = roleChangeFailure(current.role, target, role, members);
    if (authorizationFailure) return actionFailure(403, authorizationFailure);
    try {
      await updateMembershipRole(membershipId, role);
      return {
        error: false,
        message:
          target.userId === current.userId
            ? 'Role updated. Refresh your session to load the new permissions.'
            : `Role updated for ${target.email}.`
      };
    } catch (error) {
      return workosFailure(error, 'The role could not be updated.');
    }
  },

  removeMember: async (event) => {
    const current = workosActor(event);
    if (!canManage(current.role)) {
      return actionFailure(403, 'Only owners and admins can remove access.');
    }
    const form = await event.request.formData();
    const membershipId = text(form, 'membership_id');
    const members = await listOrganizationMembers(current.organizationId);
    const target = members.find((member) => member.id === membershipId);
    if (!target) return actionFailure(404, 'That member is not in this workspace.');
    const authorizationFailure = removalFailure(current.role, target, members);
    if (authorizationFailure) return actionFailure(403, authorizationFailure);
    try {
      await removeMembershipAccess(membershipId);
      return { error: false, message: `Access removed for ${target.email}.` };
    } catch (error) {
      return workosFailure(error, 'Member access could not be removed.');
    }
  },

  revokeInvitation: async (event) => {
    const current = workosActor(event);
    if (!canManage(current.role)) {
      return actionFailure(403, 'Only owners and admins can revoke invitations.');
    }
    const form = await event.request.formData();
    const invitationId = text(form, 'invitation_id');
    const invitations = await listOrganizationInvitations(current.organizationId);
    if (!invitations.some((invitation) => invitation.id === invitationId)) {
      return actionFailure(404, 'That invitation is not in this workspace.');
    }
    try {
      await revokeWorkosInvitation(invitationId);
      return { error: false, message: 'Invitation revoked.' };
    } catch (error) {
      return workosFailure(error, 'The invitation could not be revoked.');
    }
  },

  resendInvitation: async (event) => {
    const current = workosActor(event);
    if (!canManage(current.role)) {
      return actionFailure(403, 'Only owners and admins can resend invitations.');
    }
    const form = await event.request.formData();
    const invitationId = text(form, 'invitation_id');
    const invitations = await listOrganizationInvitations(current.organizationId);
    if (
      !invitations.some(
        (invitation) => invitation.id === invitationId && invitation.state === 'pending'
      )
    ) {
      return actionFailure(404, 'That pending invitation is not in this workspace.');
    }
    try {
      await resendWorkosInvitation(invitationId);
      return { error: false, message: 'Invitation resent.' };
    } catch (error) {
      return workosFailure(error, 'The invitation could not be resent.');
    }
  }
};

function workosActor(event: RequestEvent): NonNullable<ReturnType<typeof actor>> {
  if (authMode !== 'workos') redirect(303, '/dashboard/settings');
  const current = actor(event);
  if (!current) redirect(303, '/login');
  return current;
}

function actor(event: RequestEvent): {
  userId: string;
  organizationId: string;
  role: string;
} | null {
  const auth = event.locals.auth;
  if (!auth?.user || !auth.organizationId || !auth.role) return null;
  return {
    userId: auth.user.id,
    organizationId: auth.organizationId,
    role: auth.role
  };
}

function canManage(role: string): boolean {
  return role === 'owner' || role === 'admin';
}

function roleChangeFailure(
  actorRole: string,
  target: SafeOrganizationMember,
  role: OrganizationRole,
  members: SafeOrganizationMember[]
): string | null {
  if (actorRole !== 'owner' && (target.role === 'owner' || role === 'owner')) {
    return 'Only an owner can manage the owner role.';
  }
  if (
    target.role === 'owner' &&
    role !== 'owner' &&
    members.filter((member) => member.status === 'active' && member.role === 'owner').length <= 1
  ) {
    return 'A workspace must keep at least one active owner.';
  }
  return null;
}

function removalFailure(
  actorRole: string,
  target: SafeOrganizationMember,
  members: SafeOrganizationMember[]
): string | null {
  if (actorRole !== 'owner' && target.role === 'owner') {
    return 'Only an owner can remove another owner.';
  }
  if (
    target.role === 'owner' &&
    members.filter((member) => member.status === 'active' && member.role === 'owner').length <= 1
  ) {
    return 'A workspace must keep at least one active owner.';
  }
  return null;
}

function text(form: FormData, key: string): string {
  const value = form.get(key);
  return typeof value === 'string' ? value.trim() : '';
}

function validEmail(value: string): boolean {
  return value.length <= 320 && /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(value);
}

function workosFailure(error: unknown, fallback: string) {
  if (error instanceof WorkOsAdminError && error.status === 409) {
    return actionFailure(409, 'That membership or invitation already exists.');
  }
  if (error instanceof WorkOsAdminError && error.status === 422) {
    return actionFailure(422, 'WorkOS rejected the requested membership change.');
  }
  return actionFailure(502, fallback);
}

function actionFailure(status: number, message: string) {
  return fail(status, { error: true, message });
}

import { fail, redirect, type RequestEvent } from '@sveltejs/kit';
import type { Actions, PageServerLoad } from './$types';
import { authMode } from '$lib/server/auth-config';
import {
  WorkOsAdminError,
  isOrganizationRole,
  listOrganizationInvitations,
  listOrganizationMembers,
  listUserMemberships,
  organizationRoles,
  removeMembershipAccess,
  resendInvitation,
  revokeInvitation,
  sendInvitation,
  updateMembershipRole,
  type OrganizationRole,
  type SafeOrganizationMember
} from '$lib/server/workos-admin';

export const load: PageServerLoad = async (event) => {
  const current = actor(event);
  if (authMode !== 'workos' || !current) redirect(303, '/dashboard/settings');
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
    canManage: canManage(current.role)
  };
};

export const actions: Actions = {
  invite: async (event) => {
    const current = actor(event);
    if (!current) redirect(303, '/login');
    if (!canManage(current.role)) return fail(403, { message: 'Only owners and admins can invite members.' });
    const form = await event.request.formData();
    const email = text(form, 'email').toLowerCase();
    const role = text(form, 'role');
    if (!validEmail(email) || !isOrganizationRole(role)) {
      return fail(400, { message: 'Enter a valid email address and role.' });
    }
    if (role === 'owner' && current.role !== 'owner') {
      return fail(403, { message: 'Only an owner can invite another owner.' });
    }
    try {
      await sendInvitation(current.organizationId, current.userId, email, role);
      return { message: `Invitation sent to ${email}.` };
    } catch (error) {
      return workosFailure(error, 'The invitation could not be sent.');
    }
  },
  updateRole: async (event) => {
    const current = actor(event);
    if (!current) redirect(303, '/login');
    if (!canManage(current.role)) return fail(403, { message: 'Only owners and admins can update roles.' });
    const form = await event.request.formData();
    const membershipId = text(form, 'membership_id');
    const role = text(form, 'role');
    if (!membershipId || !isOrganizationRole(role)) {
      return fail(400, { message: 'Choose a valid member and role.' });
    }
    const members = await listOrganizationMembers(current.organizationId);
    const target = members.find((member) => member.id === membershipId);
    if (!target) return fail(404, { message: 'That member is not in this workspace.' });
    const authorizationFailure = roleChangeFailure(current.role, target, role, members);
    if (authorizationFailure) return fail(403, { message: authorizationFailure });
    try {
      await updateMembershipRole(membershipId, role);
      return {
        message:
          target.userId === current.userId
            ? 'Role updated. Refresh your session to load the new permissions.'
            : `Role updated for ${target.email}.`
      };
    } catch (error) {
      return workosFailure(error, 'The role could not be updated.');
    }
  },
  remove: async (event) => {
    const current = actor(event);
    if (!current) redirect(303, '/login');
    if (!canManage(current.role)) return fail(403, { message: 'Only owners and admins can remove access.' });
    const form = await event.request.formData();
    const membershipId = text(form, 'membership_id');
    const members = await listOrganizationMembers(current.organizationId);
    const target = members.find((member) => member.id === membershipId);
    if (!target) return fail(404, { message: 'That member is not in this workspace.' });
    const authorizationFailure = removalFailure(current.role, target, members);
    if (authorizationFailure) return fail(403, { message: authorizationFailure });
    try {
      await removeMembershipAccess(membershipId);
      return { message: `Access removed for ${target.email}.` };
    } catch (error) {
      return workosFailure(error, 'Member access could not be removed.');
    }
  },
  revokeInvitation: async (event) => {
    const current = actor(event);
    if (!current) redirect(303, '/login');
    if (!canManage(current.role)) return fail(403, { message: 'Only owners and admins can revoke invitations.' });
    const form = await event.request.formData();
    const invitationId = text(form, 'invitation_id');
    const invitations = await listOrganizationInvitations(current.organizationId);
    if (!invitations.some((invitation) => invitation.id === invitationId)) {
      return fail(404, { message: 'That invitation is not in this workspace.' });
    }
    try {
      await revokeInvitation(invitationId);
      return { message: 'Invitation revoked.' };
    } catch (error) {
      return workosFailure(error, 'The invitation could not be revoked.');
    }
  },
  resendInvitation: async (event) => {
    const current = actor(event);
    if (!current) redirect(303, '/login');
    if (!canManage(current.role)) return fail(403, { message: 'Only owners and admins can resend invitations.' });
    const form = await event.request.formData();
    const invitationId = text(form, 'invitation_id');
    const invitations = await listOrganizationInvitations(current.organizationId);
    if (
      !invitations.some(
        (invitation) => invitation.id === invitationId && invitation.state === 'pending'
      )
    ) {
      return fail(404, { message: 'That pending invitation is not in this workspace.' });
    }
    try {
      await resendInvitation(invitationId);
      return { message: 'Invitation resent.' };
    } catch (error) {
      return workosFailure(error, 'The invitation could not be resent.');
    }
  }
};

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
    return fail(409, { message: 'That membership or invitation already exists.' });
  }
  if (error instanceof WorkOsAdminError && error.status === 422) {
    return fail(422, { message: 'WorkOS rejected the requested membership change.' });
  }
  return fail(502, { message: fallback });
}

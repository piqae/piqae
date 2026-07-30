<script lang="ts">
  import PageHeader from '$lib/components/PageHeader.svelte';

  let { data, form } = $props();
</script>

<svelte:head><title>Team and workspaces · Piqae</title></svelte:head>

<PageHeader
  title="Team and workspaces"
  description="WorkOS-backed access, roles, invitations, and isolated workspace switching."
/>

{#if form?.message}
  <div class:failure={form.error} class="notice" role={form.error ? 'alert' : 'status'}>
    {form.message}
  </div>
{/if}

<div class="page-grid">
  <section class="panel">
    <header>
      <div>
        <h2>Workspaces</h2>
        <p>Switching issues a fresh organization-bound access token.</p>
      </div>
      <a class="button" href="/onboarding">New workspace</a>
    </header>
    <div class="workspace-list">
      {#each data.workspaces as workspace}
        <form method="POST" action="/auth/switch">
          <input type="hidden" name="organization_id" value={workspace.organizationId} />
          <input type="hidden" name="return_to" value="/dashboard/settings/team" />
          <div>
            <strong>{workspace.organizationName}</strong>
            <small>{workspace.role}</small>
          </div>
          {#if workspace.organizationId === data.viewer?.organizationId}
            <span class="current">Current</span>
          {:else}
            <button class="button" type="submit">Switch</button>
          {/if}
        </form>
      {/each}
    </div>
  </section>

  <section class="panel">
    <header>
      <div>
        <h2>Members</h2>
        <p>Role changes take effect when the member refreshes their session.</p>
      </div>
      <form method="POST" action="/auth/refresh">
        <button class="button" type="submit">Refresh my session</button>
      </form>
    </header>
    <div class="member-list">
      {#each data.members as member}
        <article>
          <div class="identity">
            <span class="avatar" aria-hidden="true">{member.email.slice(0, 1).toUpperCase()}</span>
            <span>
              <strong>{member.name ?? member.email}</strong>
              {#if member.name}<small>{member.email}</small>{/if}
            </span>
          </div>
          <span class:inactive={member.status !== 'active'} class="status">{member.status}</span>
          {#if data.canManage && member.status === 'active'}
            <form class="role-form" method="POST" action="?/updateRole">
              <input type="hidden" name="membership_id" value={member.id} />
              <label class="sr-only" for={`role-${member.id}`}>Role for {member.email}</label>
              <select id={`role-${member.id}`} name="role" value={member.role}>
                {#each data.roles as role}
                  <option value={role}>{role}</option>
                {/each}
              </select>
              <button class="button" type="submit">Update</button>
            </form>
            <form
              method="POST"
              action="?/remove"
              onsubmit={(event) => {
                if (!confirm(`Remove workspace access for ${member.email}?`)) {
                  event.preventDefault();
                }
              }}
            >
              <input type="hidden" name="membership_id" value={member.id} />
              <button class="button danger" type="submit">Remove</button>
            </form>
          {:else}
            <span class="role">{member.role}</span>
          {/if}
        </article>
      {/each}
    </div>
  </section>

  {#if data.canManage}
    <section class="panel">
      <header>
        <div>
          <h2>Invite a member</h2>
          <p>Invitations expire after 14 days and are scoped to this workspace.</p>
        </div>
      </header>
      <form class="invite-form" method="POST" action="?/invite">
        <label>
          <span>Email address</span>
          <input name="email" type="email" autocomplete="email" maxlength="320" required />
        </label>
        <label>
          <span>Role</span>
          <select name="role">
            {#each data.roles as role}
              <option value={role}>{role}</option>
            {/each}
          </select>
        </label>
        <button class="button primary" type="submit">Send invitation</button>
      </form>
    </section>
  {/if}

  {#if data.invitations.length > 0}
    <section class="panel">
      <header>
        <div>
          <h2>Pending invitations</h2>
          <p>Invitation URLs and tokens are never exposed in the dashboard response.</p>
        </div>
      </header>
      <div class="invitation-list">
        {#each data.invitations as invitation}
          <article>
            <span><strong>{invitation.email}</strong><small>{invitation.role}</small></span>
            {#if data.canManage}
              <form method="POST" action="?/resendInvitation">
                <input type="hidden" name="invitation_id" value={invitation.id} />
                <button class="button" type="submit">Resend</button>
              </form>
              <form method="POST" action="?/revokeInvitation">
                <input type="hidden" name="invitation_id" value={invitation.id} />
                <button class="button danger" type="submit">Revoke</button>
              </form>
            {/if}
          </article>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .page-grid {
    max-width: 940px;
    display: grid;
    gap: 12px;
    padding-top: 18px;
  }

  .panel > header {
    min-height: 56px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border-subtle);
  }

  h2 {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
  }

  header p {
    margin: 3px 0 0;
    color: var(--text-tertiary);
    font-size: 9px;
  }

  .notice {
    max-width: 940px;
    margin-top: 14px;
    padding: 9px 12px;
    background: color-mix(in oklch, var(--success), transparent 88%);
    border: 1px solid color-mix(in oklch, var(--success), transparent 60%);
    border-radius: var(--radius-md);
    color: var(--text-primary);
    font-size: 10px;
  }

  .notice.failure {
    background: color-mix(in oklch, var(--danger), transparent 88%);
    border-color: color-mix(in oklch, var(--danger), transparent 60%);
  }

  .workspace-list,
  .member-list,
  .invitation-list {
    display: grid;
  }

  .workspace-list form,
  .member-list article,
  .invitation-list article {
    min-height: 54px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 14px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .workspace-list form:last-child,
  .member-list article:last-child,
  .invitation-list article:last-child {
    border-bottom: 0;
  }

  .workspace-list form > div,
  .identity span:last-child,
  .invitation-list article > span:first-child {
    display: grid;
    flex: 1;
    gap: 2px;
  }

  strong {
    font-size: 10px;
    font-weight: 560;
  }

  small,
  .role,
  .current,
  .status {
    color: var(--text-tertiary);
    font-size: 9px;
    text-transform: capitalize;
  }

  .current {
    color: var(--accent);
  }

  .identity {
    min-width: 220px;
    display: flex;
    align-items: center;
    gap: 9px;
    flex: 1;
  }

  .avatar {
    width: 26px;
    height: 26px;
    display: grid;
    flex: 0 0 auto;
    place-items: center;
    background: var(--surface-hover);
    border: 1px solid var(--border-subtle);
    border-radius: 50%;
    color: var(--text-secondary);
    font-size: 10px;
  }

  .status {
    min-width: 42px;
    color: var(--success);
  }

  .status.inactive {
    color: var(--text-tertiary);
  }

  .role-form {
    display: flex;
    gap: 6px;
  }

  select,
  input {
    min-height: 29px;
    box-sizing: border-box;
    padding: 0 8px;
    background: var(--surface-raised);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-md);
    color: var(--text-primary);
    font: inherit;
    font-size: 9px;
  }

  input:focus,
  select:focus {
    border-color: var(--accent);
    outline: 2px solid color-mix(in oklch, var(--accent), transparent 75%);
  }

  .button {
    min-height: 29px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0 9px;
    background: var(--surface-raised);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    cursor: pointer;
    font: inherit;
    font-size: 9px;
    white-space: nowrap;
  }

  .button:hover,
  .button:focus-visible {
    border-color: var(--border-strong);
    color: var(--text-primary);
  }

  .button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: white;
  }

  .button.danger {
    color: var(--danger);
  }

  .invite-form {
    display: grid;
    grid-template-columns: minmax(220px, 1fr) 150px auto;
    align-items: end;
    gap: 10px;
    padding: 14px;
  }

  .invite-form label {
    display: grid;
    gap: 5px;
  }

  .invite-form label span {
    color: var(--text-secondary);
    font-size: 9px;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
  }

  @media (max-width: 760px) {
    .member-list article {
      align-items: flex-start;
      flex-wrap: wrap;
    }

    .identity {
      min-width: calc(100% - 60px);
    }

    .role-form {
      width: 100%;
    }

    .role-form select {
      flex: 1;
    }

    .invite-form {
      grid-template-columns: 1fr;
    }

    .panel > header {
      align-items: flex-start;
      flex-direction: column;
    }
  }
</style>

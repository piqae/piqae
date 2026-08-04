<script lang="ts">
  import { enhance } from '$app/forms';
  import { untrack } from 'svelte';
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  import { formatUsd } from '$lib/marketing/plans';
  import type { BillingInterval } from '$lib/marketing/types';
  import {
    DataPanel,
    DefinitionList,
    Dialog,
    EmptyState,
    Field,
    SectionHeader
  } from '$lib/components/ui';

  let { data, form } = $props();

  const live = $derived(data.dashboardMode === 'live');
  const billingContext = $derived(data.billingContext);

  const sections = $derived(
    [
      { id: 'api-keys', label: 'API keys' },
      { id: 'webhooks', label: 'Webhooks' },
      ...(data.sections.team ? [{ id: 'team', label: 'Team' }] : []),
      ...(data.sections.billing ? [{ id: 'billing', label: 'Billing' }] : []),
      { id: 'printing', label: 'Printing policy' },
      { id: 'retention', label: 'Data retention' },
      { id: 'deployment', label: 'Deployment' }
    ]
  );

  // Dialog state. Each dialog owns a `attempted` flag so a stale action result
  // from a previous submission never renders as a fresh error.
  let mutationPending = $state(false);
  let copied = $state<string | null>(null);

  let apiKeyDialog = $state(false);
  let apiKeyAttempted = $state(false);
  let apiKeyDismissed = $state(false);
  let revokeKeyDialog = $state(false);
  let revokeKeyAttempted = $state(false);
  let selectedKey = $state<{ id: string; name: string; prefix: string } | null>(null);

  let webhookDialog = $state(false);
  let webhookAttempted = $state(false);
  let webhookDismissed = $state(false);
  let deleteWebhookDialog = $state(false);
  let deleteWebhookAttempted = $state(false);
  let selectedWebhook = $state<{ id: string; url: string; description: string | null } | null>(null);

  /*
   * A one-time secret is visible only for the create action that produced it.
   * It survives the initial render after a non-JS form post, but closing the
   * dialog dismisses the session for good — reopening never re-reveals it.
   */
  const apiKeyResult = $derived(
    !mutationPending &&
      form?.mutation === 'createApiKey' &&
      (apiKeyAttempted || !apiKeyDismissed)
      ? form
      : null
  );
  const webhookResult = $derived(
    !mutationPending &&
      form?.mutation === 'createWebhook' &&
      (webhookAttempted || !webhookDismissed)
      ? form
      : null
  );

  function dismissApiKeySession() {
    apiKeyAttempted = false;
    apiKeyDismissed = true;
    copied = null;
    apiKeyDialog = false;
  }

  function dismissWebhookSession() {
    webhookAttempted = false;
    webhookDismissed = true;
    copied = null;
    webhookDialog = false;
  }

  async function copy(value: string) {
    await navigator.clipboard.writeText(value);
    copied = value;
  }

  // Billing
  const initialInterval = untrack(() => billingContext.selectedInterval);
  const initialCheckoutState = untrack(() => billingContext.checkoutState);
  let interval = $state<BillingInterval>(initialInterval);
  let billingBusy = $state(false);
  let billingMessage = $state(
    initialCheckoutState === 'success'
      ? 'Checkout returned successfully. Access changes after the billing webhook confirms the subscription.'
      : ''
  );
  const pro = $derived(billingContext.pricing.plans.find((plan) => plan.plan === 'pro')!);

  async function openBilling(path: 'checkout' | 'portal') {
    billingBusy = true;
    billingMessage = '';
    try {
      const response = await fetch(`/api/billing/${path}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: path === 'checkout' ? JSON.stringify({ plan: 'pro', interval }) : '{}'
      });
      const body = (await response.json()) as { url?: string; message?: string };
      if (!response.ok || !body.url) {
        billingMessage = body.message ?? 'Managed billing is not available for this workspace.';
        return;
      }
      window.location.assign(body.url);
    } catch {
      billingMessage = 'Managed billing could not be opened. Try again or contact support.';
    } finally {
      billingBusy = false;
    }
  }

  const usagePercent = (summary: { usage: { reportedCompleteLiveJobs: number }; entitlement: { includedLiveJobs: number } | null }) =>
    summary.entitlement
      ? Math.min(
          100,
          Math.round(
            (summary.usage.reportedCompleteLiveJobs /
              Math.max(1, summary.entitlement.includedLiveJobs)) *
              100
          )
        )
      : 0;
</script>

<svelte:head>
  <title>Settings · Piqae</title>
  <meta name="robots" content="noindex,nofollow" />
</svelte:head>

<PageHeader
  title="Settings"
  description="Credentials, event delivery, access, billing, and deployment policy."
/>

{#if form?.message}
  <p class:error={form.error} class="ui-note {form.error ? 'error' : 'success'} banner" role={form.error ? 'alert' : 'status'}>
    {form.message}
  </p>
{/if}

<div class="settings">
  <nav class="section-nav" aria-label="Settings sections">
    {#each sections as section}
      <a href={`#${section.id}`}>{section.label}</a>
    {/each}
  </nav>

  <div class="sections">
    <!-- API keys -->
    <section class="panel" id="api-keys">
      <SectionHeader
        title="API keys"
        description="Scoped credentials for applications that submit and inspect print jobs. Secret values are shown once."
      >
        {#snippet actions()}
          <button
            class="button primary"
            onclick={() => {
              copied = null;
              apiKeyDialog = true;
            }}
          >
            <Icon name="plus" size={14} /> Create secret key
          </button>
        {/snippet}
      </SectionHeader>

      {#await data.apiKeys}
        <div class="loading">Loading API keys…</div>
      {:then apiKeys}
        {#if apiKeys.dataError}<DataError error={apiKeys.dataError} />{/if}
        <DataPanel minWidth="640px">
          <table class="ui-data-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Key</th>
                <th>Environment</th>
                <th>Scopes</th>
                <th>Last used</th>
                <th><span class="sr-only">Actions</span></th>
              </tr>
            </thead>
            <tbody>
              {#each apiKeys.items as key}
                <tr>
                  <td><strong>{key.name}</strong></td>
                  <td>
                    <button class="copy" onclick={() => copy(key.prefix)} title="Copy key prefix">
                      <code>{key.prefix}••••••••</code>
                      <Icon name={copied === key.prefix ? 'check' : 'copy'} size={13} />
                    </button>
                  </td>
                  <td><span class:live-key={key.environment === 'live'} class="environment">{key.environment}</span></td>
                  <td class="muted">{key.scopes.length} scopes</td>
                  <td class="muted">
                    {#if key.lastUsedAt}<RelativeTime value={key.lastUsedAt} />{:else}Never{/if}
                  </td>
                  <td class="right">
                    <button
                      class="icon-button"
                      aria-label={`Revoke ${key.name}`}
                      onclick={() => {
                        selectedKey = key;
                        revokeKeyAttempted = false;
                        revokeKeyDialog = true;
                      }}
                    >
                      <Icon name="x" size={14} />
                    </button>
                  </td>
                </tr>
              {:else}
                <tr><td colspan="6"><EmptyState message="No API keys created." compact /></td></tr>
              {/each}
            </tbody>
          </table>
        </DataPanel>
      {/await}
    </section>

    <!-- Webhooks -->
    <section class="panel" id="webhooks">
      <SectionHeader
        title="Webhooks"
        description="Signed, durable event delivery with retries. Verify Piqae-Signature before processing an event."
      >
        {#snippet actions()}
          <button
            class="button primary"
            onclick={() => {
              copied = null;
              webhookDialog = true;
            }}
          >
            <Icon name="plus" size={14} /> Add endpoint
          </button>
        {/snippet}
      </SectionHeader>

      {#await data.webhooks}
        <div class="loading">Loading endpoints…</div>
      {:then webhooks}
        {#if webhooks.dataError}<DataError error={webhooks.dataError} />{/if}
        {#each webhooks.items as webhook}
          <article class="endpoint">
            <div class="endpoint-main">
              <div class="endpoint-title">
                <strong>{webhook.description ?? 'Webhook endpoint'}</strong>
                <Status value={webhook.status} />
              </div>
              <code>{webhook.url}</code>
              <div class="events">
                {#each webhook.events as event}<span>{event}</span>{/each}
              </div>
            </div>
            <div class="delivery">
              <span>Last delivery</span>
              <strong>
                {#if webhook.lastDeliveryAt}<RelativeTime value={webhook.lastDeliveryAt} />{:else}Never{/if}
              </strong>
            </div>
            <button
              class="icon-button"
              aria-label={`Revoke ${webhook.description ?? 'webhook endpoint'}`}
              onclick={() => {
                selectedWebhook = webhook;
                deleteWebhookAttempted = false;
                deleteWebhookDialog = true;
              }}
            >
              <Icon name="x" size={14} />
            </button>
          </article>
        {:else}
          <EmptyState message="No webhook endpoints configured." compact />
        {/each}
      {/await}
    </section>

    <!-- Team -->
    {#if data.sections.team && data.team}
      <section class="panel" id="team">
        <SectionHeader
          title="Team and workspaces"
          description="WorkOS-backed access, roles, invitations, and isolated workspace switching."
        >
          {#snippet actions()}
            <form method="POST" action="/auth/refresh">
              <button class="button" type="submit">Refresh my session</button>
            </form>
          {/snippet}
        </SectionHeader>

        {#await data.team}
          <div class="loading">Loading members…</div>
        {:then team}
          {#if team}
            {#if team.dataError}<DataError error={team.dataError} />{/if}

            <div class="subsection">
              <h3>Members</h3>
              {#each team.members as member}
                <article class="member">
                  <span class="avatar" aria-hidden="true">{member.email.slice(0, 1).toUpperCase()}</span>
                  <span class="identity">
                    <strong>{member.name ?? member.email}</strong>
                    {#if member.name}<small>{member.email}</small>{/if}
                  </span>
                  <span class:inactive={member.status !== 'active'} class="member-status">{member.status}</span>
                  {#if team.canManage && member.status === 'active'}
                    <form class="role-form" method="POST" action="?/updateMemberRole">
                      <input type="hidden" name="membership_id" value={member.id} />
                      <label class="sr-only" for={`role-${member.id}`}>Role for {member.email}</label>
                      <select class="ui-select" id={`role-${member.id}`} name="role" value={member.role}>
                        {#each team.roles as role}<option value={role}>{role}</option>{/each}
                      </select>
                      <button class="button compact" type="submit">Update</button>
                    </form>
                    <form
                      method="POST"
                      action="?/removeMember"
                      onsubmit={(event) => {
                        if (!confirm(`Remove workspace access for ${member.email}?`)) {
                          event.preventDefault();
                        }
                      }}
                    >
                      <input type="hidden" name="membership_id" value={member.id} />
                      <button class="button compact danger" type="submit">Remove</button>
                    </form>
                  {:else}
                    <span class="muted">{member.role}</span>
                  {/if}
                </article>
              {/each}
            </div>

            {#if team.canManage}
              <div class="subsection">
                <h3>Invite a member</h3>
                <form class="invite" method="POST" action="?/inviteMember">
                  <Field label="Email address">
                    <input class="input" name="email" type="email" autocomplete="email" maxlength="320" required />
                  </Field>
                  <Field label="Role">
                    <select class="input" name="role">
                      {#each team.roles as role}<option value={role}>{role}</option>{/each}
                    </select>
                  </Field>
                  <button class="button primary" type="submit">Send invitation</button>
                </form>
              </div>
            {/if}

            {#if team.invitations.length > 0}
              <div class="subsection">
                <h3>Pending invitations</h3>
                {#each team.invitations as invitation}
                  <article class="invitation">
                    <span class="identity">
                      <strong>{invitation.email}</strong>
                      <small>{invitation.role}</small>
                    </span>
                    {#if team.canManage}
                      <form method="POST" action="?/resendInvitation">
                        <input type="hidden" name="invitation_id" value={invitation.id} />
                        <button class="button compact" type="submit">Resend</button>
                      </form>
                      <form method="POST" action="?/revokeInvitation">
                        <input type="hidden" name="invitation_id" value={invitation.id} />
                        <button class="button compact danger" type="submit">Revoke</button>
                      </form>
                    {/if}
                  </article>
                {/each}
              </div>
            {/if}

            <div class="subsection">
              <h3>Workspaces</h3>
              {#each team.workspaces as workspace}
                <form class="workspace" method="POST" action="/auth/switch">
                  <input type="hidden" name="organization_id" value={workspace.organizationId} />
                  <input type="hidden" name="return_to" value="/dashboard/settings" />
                  <span class="identity">
                    <strong>{workspace.organizationName}</strong>
                    <small>{workspace.role}</small>
                  </span>
                  {#if workspace.organizationId === data.viewer?.organizationId}
                    <span class="muted">Current</span>
                  {:else}
                    <button class="button compact" type="submit">Switch</button>
                  {/if}
                </form>
              {/each}
            </div>
          {/if}
        {/await}
      </section>
    {/if}

    <!-- Billing -->
    {#if data.sections.billing && data.billing}
      <section class="panel" id="billing">
        <SectionHeader
          title="Billing"
          description="Managed Cloud plan and immutable reported-complete usage. Stripe confirms the final amount before purchase."
        />

        {#if billingMessage}
          <p class="ui-note neutral inset" role="status">{billingMessage}</p>
        {/if}

        {#await data.billing}
          <div class="loading">Loading billing…</div>
        {:then billing}
          {#if billing}
            {#if billing.dataError}<DataError error={billing.dataError} />{/if}
            <div class="billing">
              {#if billing.summary}
                <div class="plan">
                  <div class="plan-head">
                    <strong>{billing.summary.plan === 'pro' ? 'Pro' : 'Free'}</strong>
                    <span>
                      {billing.summary.subscriptionStatus ?? 'No paid subscription'}
                      {#if billing.summary.billingInterval} · {billing.summary.billingInterval}{/if}
                    </span>
                  </div>

                  <div class="meter-row">
                    <span>Reported-complete Live jobs</span>
                    <strong class="numeric">
                      {billing.summary.usage.reportedCompleteLiveJobs.toLocaleString('en-US')}
                      {#if billing.summary.entitlement}
                        / {billing.summary.entitlement.includedLiveJobs.toLocaleString('en-US')}
                      {/if}
                    </strong>
                  </div>
                  <div class="meter" aria-label={`${usagePercent(billing.summary)}% of included jobs used`}>
                    <span style={`width: ${usagePercent(billing.summary)}%`}></span>
                  </div>

                  <DefinitionList
                    items={[
                      {
                        term: 'Active nodes',
                        value: billing.summary.entitlement
                          ? `${billing.summary.usage.activeNodes} / ${billing.summary.entitlement.nodeLimit}`
                          : String(billing.summary.usage.activeNodes)
                      },
                      {
                        term: 'New Cloud jobs',
                        value: billing.summary.acceptNewCloudJobs ? 'Accepted' : 'Blocked'
                      },
                      ...(billing.summary.overageLiveJobs > 0
                        ? [
                            {
                              term: 'Overage jobs',
                              value: billing.summary.overageLiveJobs.toLocaleString('en-US')
                            }
                          ]
                        : []),
                      ...(billing.workspaceUsage && billing.summary.managedByPlatform
                        ? [
                            {
                              term: 'This workspace’s jobs',
                              value:
                                billing.workspaceUsage.reportedCompleteLiveJobs.toLocaleString('en-US')
                            }
                          ]
                        : [])
                    ]}
                  />

                  <footer class="plan-footer">
                    <span>
                      {billing.summary.managedByPlatform
                        ? 'This workspace is billed through its owning platform.'
                        : 'Taxes and the final charge are shown by Stripe.'}
                    </span>
                    {#if billingContext.canManageBilling && billingContext.portalAvailable && !billing.summary.managedByPlatform && billing.summary.plan === 'pro'}
                      <button class="button" disabled={billingBusy} onclick={() => openBilling('portal')}>
                        {billingBusy ? 'Opening…' : 'Manage billing'}
                      </button>
                    {:else if billingContext.canManageBilling && !billing.summary.managedByPlatform && billing.summary.plan === 'free'}
                      {#if billingContext.checkoutAvailable[interval]}
                        <button class="button primary" disabled={billingBusy} onclick={() => openBilling('checkout')}>
                          {billingBusy ? 'Opening…' : 'Upgrade to Pro'}
                        </button>
                      {:else}
                        <span>Checkout for this billing interval is not configured.</span>
                      {/if}
                    {:else if !billing.summary.managedByPlatform && !billingContext.canManageBilling}
                      <span>Only workspace owners and billing members can change the plan.</span>
                    {:else if billingContext.canManageBilling && !billing.summary.managedByPlatform}
                      <span>Managed billing is not configured for this deployment. Contact support.</span>
                    {/if}
                  </footer>
                </div>
              {:else}
                <p class="ui-note neutral">
                  Billing projection is temporarily unavailable. No billing action is safe while
                  status is unknown.
                </p>
              {/if}

              <aside class="plan-detail">
                <h3>Pro <small>Catalog {billingContext.pricing.version}</small></h3>
                <p><strong>{formatUsd(pro.monthlyCents)} monthly · {formatUsd(pro.annualCents)} annually</strong></p>
                <p>
                  {(interval === 'annual'
                    ? (pro.annualIncludedReportedCompleteJobs ?? pro.includedReportedCompleteJobs * 12)
                    : pro.includedReportedCompleteJobs
                  )?.toLocaleString('en-US')}
                  reported-complete jobs per {interval === 'annual' ? 'annual billing period' : 'month'}
                  and {pro.includedNodes} nodes.
                </p>
                <p><strong>{formatUsd(pro.jobOverageCents ?? 0)} per 1,000 extra jobs</strong></p>
                <p>
                  Reported completion is billable once. It is the strongest available system signal,
                  not independent proof of physical delivery.
                </p>
                <div class="ui-segmented" role="group" aria-label="Billing interval">
                  <button type="button" aria-pressed={interval === 'monthly'} onclick={() => (interval = 'monthly')}>Monthly</button>
                  <button type="button" aria-pressed={interval === 'annual'} onclick={() => (interval = 'annual')}>Annual</button>
                </div>
              </aside>
            </div>
          {/if}
        {/await}
      </section>
    {/if}

    <!-- Printing policy -->
    <section class="panel" id="printing">
      <SectionHeader
        title="Printing policy"
        description="Guardrails applied before a job can enter a local queue."
      />
      <div class="toggles">
        <label>
          <span><strong>Allow RAW printing</strong><small>Permit unrendered printer-language payloads.</small></span>
          <input type="checkbox" checked disabled />
        </label>
        <label>
          <span><strong>Allow private URI sources</strong><small>Nodes may fetch documents from private network ranges.</small></span>
          <input type="checkbox" checked disabled />
        </label>
        <label>
          <span><strong>Require manual uncertain resolution</strong><small>Never retry when OS handoff cannot be proven.</small></span>
          <input type="checkbox" checked disabled />
        </label>
      </div>
    </section>

    <!-- Data retention -->
    <section class="panel" id="retention">
      <SectionHeader
        title="Data retention"
        description="Private-beta policy preview. Automated deletion enforcement is still a release gate."
      />
      <div class="inset-form">
        <Field label="Delete successful job content after">
          <select class="input" disabled>
            <option value="1">1 hour</option>
            <option value="24" selected>24 hours</option>
            <option value="72">3 days</option>
            <option value="168">7 days</option>
          </select>
        </Field>
        <button class="button primary" disabled title="Retention mutation is not implemented">
          Save retention
        </button>
      </div>
    </section>

    <!-- Deployment -->
    <section class="panel" id="deployment">
      <SectionHeader
        title="Deployment"
        description="Capabilities reported by this control plane through GET /v1/meta."
      />
      <div class="inset">
        <DefinitionList
          columns={2}
          items={[
            { term: 'Mode', value: data.meta.deployment.replace('_', ' ') },
            { term: 'Authentication', value: data.meta.auth.provider.replace('_', ' ') },
            { term: 'Version', value: data.meta.version, mono: true },
            {
              term: 'Workspace switching',
              value: data.meta.auth.workspaceSwitching ? 'Available' : 'Single workspace'
            },
            {
              term: 'Updates',
              value: data.meta.updates.officialFeed
                ? 'Official feed'
                : data.meta.updates.customFeed
                  ? 'Custom feed'
                  : 'Manual'
            },
            { term: 'Documentation', render: docsLink }
          ]}
        />
      </div>
    </section>
  </div>
</div>

{#snippet docsLink()}
  <a href="/docs">Open docs</a>
{/snippet}

<!-- Create API key -->
<Dialog
  bind:open={apiKeyDialog}
  labelledBy="create-api-key-title"
  title="Create secret key"
  description="Grant only the capabilities this integration needs."
  onclose={dismissApiKeySession}
>
  <div class="ui-dialog__body">
    {#if !live}
      <p class="ui-note warning">Demo mode: preview only. No credential will be created.</p>
    {/if}

    <form
      id="create-api-key-form"
      method="POST"
      action="?/createApiKey"
      use:enhance={() => {
        mutationPending = true;
        apiKeyAttempted = true;
        copied = null;
        return async ({ update }) => {
          await update({ reset: false });
          mutationPending = false;
        };
      }}
    >
      <Field label="Key name">
        <input class="input" name="name" minlength="2" maxlength="120" required placeholder="Production orders" />
      </Field>
      <Field label="Expiry (optional)">
        <input class="input" name="expires_at" type="datetime-local" />
      </Field>
      <fieldset>
        <legend>Scopes</legend>
        <div class="scopes">
          <label><input type="checkbox" name="scopes" value="jobs_read" checked /> Read jobs</label>
          <label><input type="checkbox" name="scopes" value="jobs_write" checked /> Submit/cancel jobs</label>
          <label><input type="checkbox" name="scopes" value="printers_read" checked /> Read printers</label>
          <label><input type="checkbox" name="scopes" value="agents_read" /> Read nodes</label>
          <label><input type="checkbox" name="scopes" value="webhooks_read" /> Read webhooks</label>
          <label><input type="checkbox" name="scopes" value="webhooks_write" /> Manage webhooks</label>
          <label><input type="checkbox" name="scopes" value="usage_read" /> Read usage</label>
          <label><input type="checkbox" name="scopes" value="audit_read" /> Read audit log</label>
        </div>
      </fieldset>
    </form>

    {#if apiKeyResult?.error}
      <p class="ui-note error" role="alert">{apiKeyResult.error.message}</p>
    {/if}
    {#if apiKeyResult?.apiKey}
      <section class="secret" aria-live="polite">
        <div><strong>Secret key · shown once</strong><span>{apiKeyResult.apiKey.name}</span></div>
        <code>{apiKeyResult.apiKey.secret}</code>
        <button class="button compact" type="button" onclick={() => copy(apiKeyResult.apiKey.secret)}>
          <Icon name="copy" size={13} /> {copied === apiKeyResult.apiKey.secret ? 'Copied' : 'Copy key'}
        </button>
      </section>
    {/if}
  </div>

  {#snippet footer()}
    <button class="button" type="button" aria-label="Close secret key dialog" onclick={dismissApiKeySession}>
      Close
    </button>
    <button class="button primary" type="submit" form="create-api-key-form" disabled={mutationPending || !live}>
      {mutationPending ? 'Creating…' : 'Create secret key'}
    </button>
  {/snippet}
</Dialog>

<!-- Revoke API key -->
<Dialog
  bind:open={revokeKeyDialog}
  labelledBy="revoke-api-key-title"
  title="Revoke this API key?"
  description="Requests using this credential will stop authenticating immediately."
>
  <div class="ui-dialog__body">
    <form
      id="revoke-api-key-form"
      method="POST"
      action="?/revokeApiKey"
      use:enhance={() => {
        mutationPending = true;
        revokeKeyAttempted = true;
        return async ({ result, update }) => {
          await update();
          mutationPending = false;
          if (result.type === 'success') revokeKeyDialog = false;
        };
      }}
    >
      <input type="hidden" name="api_key_id" value={selectedKey?.id ?? ''} />
      <p class="ui-note neutral">
        Revoke <strong>{selectedKey?.name}</strong> (<code>{selectedKey?.prefix}••••••</code>)? This
        cannot be undone.
      </p>
    </form>
    {#if !live}<p class="ui-note warning">Demo mode: no credential will be revoked.</p>{/if}
    {#if revokeKeyAttempted && !mutationPending && form?.mutation === 'revokeApiKey' && form?.error}
      <p class="ui-note error" role="alert">{form.error.message}</p>
    {/if}
  </div>

  {#snippet footer()}
    <button class="button" type="button" onclick={() => (revokeKeyDialog = false)}>Keep key</button>
    <button class="button danger-solid" type="submit" form="revoke-api-key-form" disabled={mutationPending || !live}>
      {mutationPending ? 'Revoking…' : 'Revoke key'}
    </button>
  {/snippet}
</Dialog>

<!-- Create webhook -->
<Dialog
  bind:open={webhookDialog}
  labelledBy="create-webhook-title"
  title="Add webhook endpoint"
  description="Events are signed and retried until Piqae receives a successful response."
  onclose={dismissWebhookSession}
>
  <div class="ui-dialog__body">
    {#if !live}
      <p class="ui-note warning">Demo mode: preview only. No endpoint will be created.</p>
    {/if}

    <form
      id="create-webhook-form"
      method="POST"
      action="?/createWebhook"
      use:enhance={() => {
        mutationPending = true;
        webhookAttempted = true;
        copied = null;
        return async ({ update }) => {
          await update({ reset: false });
          mutationPending = false;
        };
      }}
    >
      <Field label="Endpoint URL">
        <input class="input" name="url" type="url" required placeholder="https://example.com/piqae/events" />
      </Field>
      <fieldset>
        <legend>Event families</legend>
        <div class="scopes">
          <label><input type="checkbox" name="events" value="job.*" checked /> Jobs</label>
          <label><input type="checkbox" name="events" value="agent.*" /> Nodes</label>
          <label><input type="checkbox" name="events" value="printer.*" /> Printers</label>
        </div>
      </fieldset>
    </form>

    {#if webhookResult?.error}
      <p class="ui-note error" role="alert">{webhookResult.error.message}</p>
    {/if}
    {#if webhookResult?.webhook}
      <section class="secret" aria-live="polite">
        <div><strong>Signing secret · shown once</strong><span>{webhookResult.webhook.url}</span></div>
        <code>{webhookResult.webhook.secret}</code>
        <button class="button compact" type="button" onclick={() => copy(webhookResult.webhook.secret)}>
          <Icon name="copy" size={13} /> {copied === webhookResult.webhook.secret ? 'Copied' : 'Copy secret'}
        </button>
      </section>
    {/if}
  </div>

  {#snippet footer()}
    <button class="button" type="button" aria-label="Close webhook dialog" onclick={dismissWebhookSession}>
      Close
    </button>
    <button class="button primary" type="submit" form="create-webhook-form" disabled={mutationPending || !live}>
      {mutationPending ? 'Creating…' : 'Create endpoint'}
    </button>
  {/snippet}
</Dialog>

<!-- Revoke webhook -->
<Dialog
  bind:open={deleteWebhookDialog}
  labelledBy="delete-webhook-title"
  title="Revoke webhook endpoint?"
  description="Piqae will stop sending new deliveries to this endpoint."
>
  <div class="ui-dialog__body">
    <form
      id="delete-webhook-form"
      method="POST"
      action="?/deleteWebhook"
      use:enhance={() => {
        mutationPending = true;
        deleteWebhookAttempted = true;
        return async ({ result, update }) => {
          await update();
          mutationPending = false;
          if (result.type === 'success') deleteWebhookDialog = false;
        };
      }}
    >
      <input type="hidden" name="webhook_id" value={selectedWebhook?.id ?? ''} />
      <p class="ui-note neutral">
        Revoke <strong>{selectedWebhook?.description ?? 'this webhook endpoint'}</strong> at
        <code>{selectedWebhook?.url}</code>? This cannot be undone.
      </p>
    </form>
    {#if !live}<p class="ui-note warning">Demo mode: no endpoint will be revoked.</p>{/if}
    {#if deleteWebhookAttempted && !mutationPending && form?.mutation === 'deleteWebhook' && form?.error}
      <p class="ui-note error" role="alert">{form.error.message}</p>
    {/if}
  </div>

  {#snippet footer()}
    <button class="button" type="button" onclick={() => (deleteWebhookDialog = false)}>Keep endpoint</button>
    <button class="button danger-solid" type="submit" form="delete-webhook-form" disabled={mutationPending || !live}>
      {mutationPending ? 'Revoking…' : 'Revoke endpoint'}
    </button>
  {/snippet}
</Dialog>

<style>
  .banner {
    margin: 16px 0 0;
  }

  .settings {
    display: grid;
    grid-template-columns: 168px minmax(0, 1fr);
    align-items: start;
    gap: 28px;
    padding-top: 20px;
  }

  .section-nav {
    position: sticky;
    top: calc(var(--topbar-height) + 16px);
    display: grid;
    gap: 1px;
  }

  .section-nav a {
    height: var(--control-compact);
    display: flex;
    align-items: center;
    padding: 0 10px;
    color: var(--text-secondary);
    border-radius: var(--radius-md);
    font-size: var(--text-compact);
  }

  .section-nav a:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .sections {
    display: grid;
    gap: 14px;
    max-width: 820px;
  }

  section {
    scroll-margin-top: calc(var(--topbar-height) + 16px);
  }

  .loading {
    padding: 20px 16px;
    color: var(--text-tertiary);
    font-size: var(--text-compact);
  }

  .inset,
  .inset-form {
    padding: 14px 16px;
  }

  .inset-form {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
  }

  .inset-form :global(.field) {
    max-width: 300px;
    flex: 1;
  }

  .ui-note.inset {
    margin: 14px 16px 0;
  }

  /* API keys */
  .copy {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 0;
    color: var(--text-secondary);
    background: none;
    border: 0;
    cursor: pointer;
    font: inherit;
  }

  .copy:hover {
    color: var(--text-primary);
  }

  .copy code {
    font-family: var(--font-mono);
    font-size: var(--text-meta);
  }

  .environment {
    padding: 2px 7px;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: 99px;
    font-size: var(--text-meta);
    text-transform: capitalize;
  }

  .environment.live-key {
    color: var(--success);
    background: var(--success-soft);
    border-color: transparent;
  }

  /* Webhooks */
  .endpoint {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .endpoint:last-child {
    border-bottom: 0;
  }

  .endpoint-main {
    min-width: 0;
    flex: 1;
    display: grid;
    gap: 5px;
  }

  .endpoint-title {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: var(--text-compact);
  }

  .endpoint-title strong {
    font-weight: 530;
  }

  .endpoint code {
    overflow: hidden;
    color: var(--text-secondary);
    font-family: var(--font-mono);
    font-size: var(--text-meta);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .events {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .events span {
    padding: 2px 7px;
    color: var(--text-tertiary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: var(--text-meta);
  }

  .delivery {
    flex: 0 0 auto;
    display: grid;
    gap: 2px;
    text-align: right;
    font-size: var(--text-meta);
  }

  .delivery span {
    color: var(--text-tertiary);
  }

  .delivery strong {
    color: var(--text-secondary);
    font-weight: 500;
  }

  /* Team */
  .subsection {
    padding: 14px 16px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .subsection:last-child {
    border-bottom: 0;
  }

  h3 {
    margin: 0 0 10px;
    color: var(--text-secondary);
    font-size: var(--text-compact);
    font-weight: 550;
  }

  .member,
  .invitation,
  .workspace {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 0;
    border-bottom: 1px solid var(--border-subtle);
    font-size: var(--text-compact);
  }

  .member:last-of-type,
  .invitation:last-of-type,
  .workspace:last-of-type {
    border-bottom: 0;
  }

  .avatar {
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    flex: 0 0 auto;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: 50%;
    font-size: var(--text-meta);
    font-weight: 550;
  }

  .identity {
    min-width: 0;
    flex: 1;
    display: grid;
    line-height: var(--text-compact-line);
  }

  .identity strong {
    overflow: hidden;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .identity small {
    color: var(--text-tertiary);
    font-size: var(--text-meta);
  }

  .member-status {
    color: var(--success);
    font-size: var(--text-meta);
    text-transform: capitalize;
  }

  .member-status.inactive {
    color: var(--text-tertiary);
  }

  .role-form {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .invite {
    display: flex;
    align-items: flex-end;
    gap: 12px;
    flex-wrap: wrap;
  }

  .invite :global(.field) {
    min-width: 200px;
    flex: 1;
  }

  .button.danger {
    color: var(--danger);
  }

  /* Billing */
  .billing {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 280px;
    align-items: start;
    gap: 20px;
    padding: 16px;
  }

  .plan {
    display: grid;
    gap: 12px;
  }

  .plan-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }

  .plan-head strong {
    font-size: 17px;
    font-weight: 560;
    letter-spacing: -0.02em;
  }

  .plan-head span {
    color: var(--text-secondary);
    font-size: var(--text-meta);
    text-transform: capitalize;
  }

  .meter-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    font-size: var(--text-compact);
  }

  .meter-row span {
    color: var(--text-secondary);
  }

  .meter {
    height: 6px;
    overflow: hidden;
    background: var(--surface-raised);
    border-radius: 99px;
  }

  .meter span {
    display: block;
    height: 100%;
    background: var(--accent);
    border-radius: 99px;
  }

  .plan-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding-top: 4px;
    color: var(--text-tertiary);
    font-size: var(--text-meta);
    line-height: var(--text-meta-line);
  }

  .plan-detail {
    padding: 14px;
    background: var(--canvas);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
  }

  .plan-detail h3 {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
    font-size: var(--text-section);
  }

  .plan-detail h3 small {
    color: var(--text-tertiary);
    font-size: var(--text-meta);
    font-weight: 450;
  }

  .plan-detail p {
    margin: 0 0 9px;
    color: var(--text-secondary);
    font-size: var(--text-meta);
    line-height: var(--text-meta-line);
  }

  .plan-detail strong {
    color: var(--text-primary);
    font-weight: 530;
  }

  /* Policy toggles */
  .toggles label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .toggles label:last-child {
    border-bottom: 0;
  }

  .toggles label > span {
    display: grid;
    gap: 2px;
  }

  .toggles strong {
    font-size: var(--text-compact);
    font-weight: 520;
  }

  .toggles small {
    color: var(--text-tertiary);
    font-size: var(--text-meta);
  }

  input[type='checkbox'] {
    width: 32px;
    height: 18px;
    flex: 0 0 auto;
    appearance: none;
    background: var(--surface-raised);
    border: 1px solid var(--border-default);
    border-radius: 99px;
    cursor: pointer;
  }

  input[type='checkbox']::after {
    width: 12px;
    height: 12px;
    display: block;
    margin: 2px;
    content: '';
    background: var(--text-tertiary);
    border-radius: 50%;
    transition: transform 100ms ease;
  }

  input[type='checkbox']:checked {
    background: var(--accent);
    border-color: var(--accent);
  }

  input[type='checkbox']:checked::after {
    background: white;
    transform: translateX(14px);
  }

  /* Dialog internals */
  form {
    display: grid;
    gap: 12px;
  }

  fieldset {
    margin: 0;
    padding: 10px 12px;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
  }

  legend {
    padding: 0 5px;
    color: var(--text-tertiary);
    font-size: var(--text-meta);
  }

  .scopes {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px 14px;
  }

  .scopes label {
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--text-secondary);
    font-size: var(--text-compact);
  }

  .scopes input[type='checkbox'] {
    width: 15px;
    height: 15px;
    appearance: auto;
    border-radius: var(--radius-sm);
  }

  .scopes input[type='checkbox']::after {
    display: none;
  }

  .secret {
    display: grid;
    gap: 8px;
    padding: 12px;
    background: var(--success-soft);
    border: 1px solid color-mix(in oklch, var(--success), transparent 72%);
    border-radius: var(--radius-md);
  }

  .secret > div {
    display: flex;
    justify-content: space-between;
    gap: 10px;
  }

  .secret strong {
    color: var(--success);
    font-size: var(--text-compact);
    font-weight: 550;
  }

  .secret span {
    color: var(--text-tertiary);
    font-size: var(--text-meta);
  }

  .secret code {
    overflow-wrap: anywhere;
    color: var(--text-secondary);
    font: var(--text-code) / var(--text-code-line) var(--font-mono);
  }

  .secret .button {
    justify-self: start;
  }

  @media (max-width: 900px) {
    .settings {
      grid-template-columns: 1fr;
      gap: 14px;
    }

    .section-nav {
      position: static;
      display: flex;
      overflow-x: auto;
      gap: 4px;
    }

    .section-nav a {
      white-space: nowrap;
    }

    .billing {
      grid-template-columns: 1fr;
    }
  }

  @media (max-width: 620px) {
    .endpoint,
    .member,
    .invitation {
      align-items: flex-start;
      flex-wrap: wrap;
    }

    .scopes,
    .inset-form {
      grid-template-columns: 1fr;
      flex-direction: column;
      align-items: stretch;
    }
  }
</style>

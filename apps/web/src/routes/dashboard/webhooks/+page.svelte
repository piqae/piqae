<script lang="ts">
  import { enhance } from '$app/forms';
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  let { data, form } = $props();
  const webhooks = $derived(data.webhooks);

  let createDialog: HTMLDialogElement;
  let deleteDialog: HTMLDialogElement;
  let selectedWebhook = $state<(typeof webhooks)[number] | null>(null);
  let mutationPending = $state(false);
  let copied = $state(false);
  let createAttemptSubmitted = $state(false);
  let createSessionDismissed = $state(false);
  const createResultVisible = $derived(
    createAttemptSubmitted ||
      (!createSessionDismissed && form?.mutation === 'createWebhook')
  );
  let deleteAttemptVisible = $state(false);

  function confirmDelete(webhook: (typeof webhooks)[number]) {
    selectedWebhook = webhook;
    deleteAttemptVisible = false;
    deleteDialog.showModal();
  }

  function openCreate() {
    copied = false;
    createDialog.showModal();
  }

  function resetCreateSession() {
    createAttemptSubmitted = false;
    createSessionDismissed = true;
    copied = false;
  }

  function closeCreate() {
    resetCreateSession();
    createDialog.close();
  }

  function resetDeleteSession() {
    deleteAttemptVisible = false;
  }

  function closeDelete() {
    resetDeleteSession();
    deleteDialog.close();
  }

  async function copySecret(secret: string) {
    await navigator.clipboard.writeText(secret);
    copied = true;
  }
</script>

<svelte:head><title>Webhooks · Spool</title></svelte:head>

{#snippet actions()}
  <button class="button primary" onclick={openCreate}><Icon name="plus" size={13} /> Add endpoint</button>
{/snippet}

<PageHeader
  title="Webhooks"
  description="Signed, durable event delivery with retries and replay."
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

<section class="notice">
  <Icon name="bolt" size={14} />
  <p>
    Spool signs the exact request body using HMAC-SHA256. Always verify
    <code>Spool-Signature</code> before processing an event.
  </p>
  <a href="/docs/webhooks">Read guide <Icon name="arrow-right" size={11} /></a>
</section>

<div class="panel endpoints">
  <header>
    <div>
      <h2>Endpoints</h2>
      <span>{webhooks.length} configured</span>
    </div>
  </header>
  {#each webhooks as webhook}
    <article>
      <span class="endpoint-icon"><Icon name="webhooks" size={15} /></span>
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
        aria-label={`Revoke ${webhook.description ?? 'webhook endpoint'}`}
        onclick={() => confirmDelete(webhook)}
      ><Icon name="x" size={14} /></button>
    </article>
  {/each}
  {#if webhooks.length === 0 && !data.dataError}
    <div class="empty-state">No webhook endpoints configured.</div>
  {/if}
</div>

{#if data.dashboardMode === 'demo'}
<section class="panel attempts" aria-label="Demo webhook delivery examples">
  <header>
    <div>
      <h2>Demo delivery examples</h2>
      <span>Illustrative data — not control-plane evidence</span>
    </div>
    <span class="demo-label">Demo only</span>
  </header>
  <table>
    <thead>
      <tr><th>Event</th><th>Endpoint</th><th>Response</th><th>Attempt</th><th class="right">Time</th></tr>
    </thead>
    <tbody>
      <tr>
        <td><code>job.completed_reported</code></td>
        <td class="muted">Order status updates</td>
        <td><span class="http success">200</span></td>
        <td class="numeric muted">1</td>
        <td class="right muted">1m ago</td>
      </tr>
      <tr>
        <td><code>printer.state_changed</code></td>
        <td class="muted">Fleet health</td>
        <td><span class="http danger">503</span></td>
        <td class="numeric muted">4</td>
        <td class="right muted">11m ago</td>
      </tr>
      <tr>
        <td><code>job.failed_terminal</code></td>
        <td class="muted">Order status updates</td>
        <td><span class="http success">204</span></td>
        <td class="numeric muted">1</td>
        <td class="right muted">18m ago</td>
      </tr>
    </tbody>
  </table>
</section>
{:else}
  <section class="panel delivery-unavailable">
    <Icon name="activity" size={15} />
    <div>
      <strong>Delivery history is not connected yet</strong>
      <p>Configured endpoints above are live. Attempt history will appear when the dashboard integrates the delivery endpoint.</p>
    </div>
  </section>
{/if}

<dialog bind:this={createDialog} aria-labelledby="create-webhook-title" onclose={resetCreateSession}>
  <form
    method="POST"
    action="?/createWebhook"
    use:enhance={() => {
      mutationPending = true;
      createAttemptSubmitted = true;
      createSessionDismissed = false;
      copied = false;
      return async ({ update }) => {
        await update({ reset: false });
        mutationPending = false;
      };
    }}
  >
    <header class="dialog-header">
      <div>
        <h2 id="create-webhook-title">Add webhook endpoint</h2>
        <p>Events are signed and retried until Spool receives a successful response.</p>
      </div>
      <button class="icon-button" type="button" aria-label="Close webhook dialog" onclick={closeCreate}>×</button>
    </header>

    {#if data.dashboardMode === 'demo'}
      <p class="demo-note">Demo mode: preview only. No endpoint will be created.</p>
    {/if}

    <div class="dialog-body">
      <label class="field">
        <span>Endpoint URL</span>
        <input name="url" type="url" required placeholder="https://example.com/spool/events" />
      </label>
      <fieldset>
        <legend>Event families</legend>
        <label><input type="checkbox" name="events" value="job.*" checked /> Jobs</label>
        <label><input type="checkbox" name="events" value="agent.*" /> Agents</label>
        <label><input type="checkbox" name="events" value="printer.*" /> Printers</label>
      </fieldset>

      {#if createResultVisible && !mutationPending && form?.mutation === 'createWebhook' && form?.error}
        <p class="form-message error" role="alert">{form.error.message}</p>
      {/if}

      {#if createResultVisible && !mutationPending && form?.mutation === 'createWebhook' && form?.webhook}
        <section class="secret-result" aria-live="polite">
          <div>
            <strong>Signing secret · shown once</strong>
            <span>{form.webhook.url}</span>
          </div>
          <code>{form.webhook.secret}</code>
          <button class="button" type="button" onclick={() => copySecret(form.webhook.secret)}>
            <Icon name="copy" size={12} /> {copied ? 'Copied' : 'Copy secret'}
          </button>
        </section>
      {/if}
    </div>

    <footer class="dialog-footer">
      <button class="button" type="button" onclick={closeCreate}>Close</button>
      <button
        class="button primary"
        type="submit"
        disabled={mutationPending || data.dashboardMode !== 'live'}
      >{mutationPending ? 'Creating…' : 'Create endpoint'}</button>
    </footer>
  </form>
</dialog>

<dialog bind:this={deleteDialog} aria-labelledby="delete-webhook-title" onclose={resetDeleteSession}>
  <form
    method="POST"
    action="?/deleteWebhook"
    use:enhance={() => {
      mutationPending = true;
      deleteAttemptVisible = true;
      return async ({ result, update }) => {
        await update();
        mutationPending = false;
        if (result.type === 'success') closeDelete();
      };
    }}
  >
    <header class="dialog-header">
      <div>
        <h2 id="delete-webhook-title">Revoke webhook endpoint?</h2>
        <p>Spool will stop sending new deliveries to this endpoint.</p>
      </div>
      <button class="icon-button" type="button" aria-label="Close revoke dialog" onclick={closeDelete}>×</button>
    </header>
    <div class="dialog-body">
      <input type="hidden" name="webhook_id" value={selectedWebhook?.id ?? ''} />
      <p class="confirm-copy">
        Revoke <strong>{selectedWebhook?.description ?? 'this webhook endpoint'}</strong> at
        <code>{selectedWebhook?.url}</code>? This cannot be undone.
      </p>
      {#if data.dashboardMode === 'demo'}
        <p class="demo-note inline">Demo mode: no endpoint will be revoked.</p>
      {/if}
      {#if deleteAttemptVisible && !mutationPending && form?.mutation === 'deleteWebhook' && form?.error}
        <p class="form-message error" role="alert">{form.error.message}</p>
      {/if}
    </div>
    <footer class="dialog-footer">
      <button class="button" type="button" onclick={closeDelete}>Keep endpoint</button>
      <button
        class="button danger-button"
        type="submit"
        disabled={mutationPending || data.dashboardMode !== 'live'}
      >{mutationPending ? 'Revoking…' : 'Revoke endpoint'}</button>
    </footer>
  </form>
</dialog>

<style>
  .notice {
    min-height: 43px;
    display: flex;
    align-items: center;
    gap: 9px;
    margin: 13px 0;
    padding: 7px 10px;
    color: var(--info);
    background: var(--info-soft);
    border: 1px solid color-mix(in oklch, var(--info), transparent 75%);
    border-radius: var(--radius-md);
  }

  .notice p {
    flex: 1;
    margin: 0;
    color: var(--text-secondary);
    font-size: 10px;
  }

  code {
    font: 9px/15px var(--font-mono);
  }

  .notice a {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 9px;
    white-space: nowrap;
  }

  .panel > header {
    height: 49px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  header > div {
    display: grid;
  }

  h2 {
    margin: 0;
    font-size: 11px;
    font-weight: 550;
  }

  header span {
    color: var(--text-tertiary);
    font-size: 9px;
    line-height: 14px;
  }

  .endpoints article {
    min-height: 76px;
    display: grid;
    grid-template-columns: 32px minmax(0, 1fr) 90px 26px;
    align-items: center;
    gap: 10px;
    padding: 9px 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .endpoints article:last-child {
    border-bottom: 0;
  }

  .endpoint-icon {
    width: 31px;
    height: 31px;
    display: grid;
    place-items: center;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: 8px;
  }

  .endpoint-main {
    min-width: 0;
    display: grid;
  }

  .endpoint-title {
    display: flex;
    align-items: center;
    gap: 9px;
  }

  .endpoint-title strong {
    font-size: 11px;
    font-weight: 520;
  }

  .endpoint-main > code {
    overflow: hidden;
    color: var(--text-tertiary);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .events {
    display: flex;
    gap: 4px;
    margin-top: 3px;
  }

  .events span {
    padding: 1px 4px;
    color: var(--text-tertiary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    font: 8px/12px var(--font-mono);
  }

  .delivery {
    display: grid;
    justify-items: end;
  }

  .delivery span {
    color: var(--text-tertiary);
    font-size: 8px;
  }

  .delivery strong {
    color: var(--text-secondary);
    font-size: 9px;
    font-weight: 450;
  }

  article > button {
    width: 25px;
    height: 25px;
    display: grid;
    place-items: center;
    color: var(--text-tertiary);
    background: transparent;
    border: 0;
    border-radius: var(--radius-sm);
  }

  article > button:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .attempts {
    margin-top: 12px;
    overflow-x: auto;
  }

  .demo-label {
    padding: 2px 6px;
    color: var(--warning);
    background: var(--warning-soft);
    border-radius: var(--radius-sm);
    font-size: 8px;
    text-transform: uppercase;
  }

  .delivery-unavailable {
    min-height: 74px;
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 12px;
    padding: 13px;
    color: var(--text-tertiary);
  }

  .delivery-unavailable strong {
    color: var(--text-secondary);
    font-size: 10px;
    font-weight: 520;
  }

  .delivery-unavailable p {
    margin: 2px 0 0;
    color: var(--text-tertiary);
    font-size: 9px;
  }

  table {
    width: 100%;
    min-width: 600px;
    border-collapse: collapse;
    font-size: 10px;
  }

  th {
    height: 29px;
    padding: 0 12px;
    color: var(--text-tertiary);
    font-size: 8px;
    font-weight: 500;
    text-align: left;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-bottom: 1px solid var(--border-subtle);
  }

  td {
    height: 39px;
    padding: 0 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  tr:last-child td {
    border-bottom: 0;
  }

  .http {
    font: 9px var(--font-mono);
  }

  .http.success {
    color: var(--success);
  }

  .http.danger {
    color: var(--danger);
  }

  .right {
    text-align: right;
  }

  dialog {
    width: min(460px, calc(100vw - 24px));
    padding: 0;
    color: var(--text-primary);
    background: var(--surface);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-lg);
    box-shadow: 0 22px 70px rgb(0 0 0 / 38%);
  }

  dialog::backdrop {
    background: rgb(7 9 13 / 65%);
    backdrop-filter: blur(2px);
  }

  dialog form {
    display: grid;
  }

  .dialog-header,
  .dialog-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 14px;
  }

  .dialog-header {
    border-bottom: 1px solid var(--border-subtle);
  }

  .dialog-header h2 {
    margin: 0;
    font-size: 12px;
    font-weight: 560;
  }

  .dialog-header p,
  .demo-note {
    margin: 3px 0 0;
    color: var(--text-tertiary);
    font-size: 9px;
    line-height: 14px;
  }

  .icon-button {
    width: 25px;
    height: 25px;
    color: var(--text-tertiary);
    background: transparent;
    border: 0;
    border-radius: var(--radius-sm);
    font-size: 17px;
  }

  .icon-button:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .demo-note {
    margin: 10px 14px 0;
    padding: 7px 8px;
    color: var(--warning);
    background: var(--warning-soft);
    border-radius: var(--radius-md);
  }

  .demo-note.inline {
    margin: 0;
  }

  .dialog-body {
    display: grid;
    gap: 12px;
    padding: 14px;
  }

  .field {
    display: grid;
    gap: 5px;
    color: var(--text-secondary);
    font-size: 9px;
  }

  .field input {
    width: 100%;
    height: 31px;
    padding: 0 9px;
    color: var(--text-primary);
    background: var(--canvas);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-md);
    outline: 0;
    font-size: 10px;
  }

  .field input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  fieldset {
    display: flex;
    gap: 13px;
    margin: 0;
    padding: 9px 10px;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
  }

  legend {
    padding: 0 4px;
    color: var(--text-tertiary);
    font-size: 8px;
  }

  fieldset label {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--text-secondary);
    font-size: 9px;
  }

  .form-message,
  .confirm-copy {
    margin: 0;
    padding: 8px;
    border-radius: var(--radius-md);
    font-size: 9px;
    line-height: 15px;
  }

  .form-message.error {
    color: var(--danger);
    background: var(--danger-soft);
  }

  .confirm-copy {
    color: var(--text-secondary);
    background: var(--canvas);
    border: 1px solid var(--border-subtle);
  }

  .confirm-copy code {
    overflow-wrap: anywhere;
  }

  .secret-result {
    display: grid;
    gap: 8px;
    padding: 10px;
    background: var(--success-soft);
    border: 1px solid color-mix(in oklch, var(--success), transparent 72%);
    border-radius: var(--radius-md);
  }

  .secret-result > div {
    display: grid;
  }

  .secret-result strong {
    color: var(--success);
    font-size: 9px;
    font-weight: 550;
  }

  .secret-result span {
    overflow: hidden;
    color: var(--text-tertiary);
    font-size: 8px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .secret-result code {
    overflow-wrap: anywhere;
    color: var(--text-secondary);
    font: 9px/15px var(--font-mono);
  }

  .secret-result .button {
    justify-self: start;
  }

  .dialog-footer {
    justify-content: flex-end;
    border-top: 1px solid var(--border-subtle);
  }

  .danger-button {
    color: white;
    background: var(--danger);
    border-color: var(--danger);
  }

  @media (max-width: 660px) {
    .notice a {
      display: none;
    }

    .endpoints article {
      grid-template-columns: 32px minmax(0, 1fr) 26px;
    }

    .delivery {
      display: none;
    }
  }
</style>

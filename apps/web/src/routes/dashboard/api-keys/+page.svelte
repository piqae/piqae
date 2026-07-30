<script lang="ts">
  import { enhance } from '$app/forms';
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  let { data, form } = $props();
  const apiKeys = $derived(data.apiKeys);
  let copied = $state<string | null>(null);
  let createDialog: HTMLDialogElement;
  let revokeDialog: HTMLDialogElement;
  let selectedApiKey = $state<(typeof apiKeys)[number] | null>(null);
  let mutationPending = $state(false);
  let createAttemptSubmitted = $state(false);
  let createSessionDismissed = $state(false);
  const createResultVisible = $derived(
    createAttemptSubmitted ||
      (!createSessionDismissed && form?.mutation === 'createApiKey')
  );
  let revokeAttemptVisible = $state(false);

  async function copyPrefix(prefix: string) {
    await navigator.clipboard.writeText(prefix);
    copied = prefix;
    setTimeout(() => (copied = null), 1200);
  }

  async function copySecret(secret: string) {
    await navigator.clipboard.writeText(secret);
    copied = secret;
  }

  function confirmRevoke(apiKey: (typeof apiKeys)[number]) {
    selectedApiKey = apiKey;
    revokeAttemptVisible = false;
    revokeDialog.showModal();
  }

  function openCreate() {
    copied = null;
    createDialog.showModal();
  }

  function resetCreateSession() {
    createAttemptSubmitted = false;
    createSessionDismissed = true;
    copied = null;
  }

  function closeCreate() {
    resetCreateSession();
    createDialog.close();
  }

  function resetRevokeSession() {
    revokeAttemptVisible = false;
  }

  function closeRevoke() {
    resetRevokeSession();
    revokeDialog.close();
  }
</script>

<svelte:head><title>API keys · Piqae</title></svelte:head>

{#snippet actions()}
  <button class="button primary" onclick={openCreate}><Icon name="plus" size={13} /> Create secret key</button>
{/snippet}

<PageHeader
  eyebrow="Developers"
  title="API keys"
  description="Scoped credentials for applications that submit and inspect print jobs."
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

<section class="security-note">
  <Icon name="api" size={15} />
  <div>
    <strong>Secret values are shown once</strong>
    <p>Piqae stores a one-way hash. Rotate a key immediately if its value is exposed.</p>
  </div>
  <a href="/docs/api-keys">Security guide <Icon name="arrow-right" size={11} /></a>
</section>

<div class="panel">
  <table>
    <thead>
      <tr><th>Name</th><th>Key</th><th>Environment</th><th>Scopes</th><th>Last used</th><th>Created</th><th></th></tr>
    </thead>
    <tbody>
      {#each apiKeys as key}
        <tr>
          <td><strong>{key.name}</strong></td>
          <td>
            <button class="copy" onclick={() => copyPrefix(key.prefix)} title="Copy key prefix">
              <code>{key.prefix}••••••••••</code>
              <Icon name={copied === key.prefix ? 'check' : 'copy'} size={11} />
            </button>
          </td>
          <td><span class:live={key.environment === 'live'} class="environment">{key.environment}</span></td>
          <td><span class="scope-count">{key.scopes.length} scopes</span></td>
          <td class="muted">{#if key.lastUsedAt}<RelativeTime value={key.lastUsedAt} />{:else}Never{/if}</td>
          <td class="muted"><RelativeTime value={key.createdAt} /></td>
          <td class="action">
            <button aria-label={`Revoke ${key.name}`} onclick={() => confirmRevoke(key)}>
              <Icon name="x" size={14} />
            </button>
          </td>
        </tr>
      {/each}
      {#if apiKeys.length === 0 && !data.dataError}
        <tr><td colspan="7"><div class="empty-state">No API keys created.</div></td></tr>
      {/if}
    </tbody>
  </table>
</div>

<section class="quickstart panel">
  <div>
    <span class="step">Next step</span>
    <h2>Send your first print job</h2>
    <p>Use the test environment until your integration is ready for physical printers.</p>
  </div>
  <pre><code><span>curl</span> https://api.piqae.com/v1/jobs \
  -H <em>"Authorization: Bearer $PIQAE_API_KEY"</em> \
  -H <em>"Idempotency-Key: order-481"</em> \
  -H <em>"Content-Type: application/json"</em> \
  -d @job.json</code></pre>
  <a class="button" href="/docs/quickstart">Open quick start <Icon name="arrow-right" size={12} /></a>
</section>

<dialog bind:this={createDialog} aria-labelledby="create-api-key-title" onclose={resetCreateSession}>
  <form
    method="POST"
    action="?/createApiKey"
    use:enhance={() => {
      mutationPending = true;
      createAttemptSubmitted = true;
      createSessionDismissed = false;
      copied = null;
      return async ({ update }) => {
        await update({ reset: false });
        mutationPending = false;
      };
    }}
  >
    <header class="dialog-header">
      <div>
        <h2 id="create-api-key-title">Create secret key</h2>
        <p>Grant only the capabilities this integration needs.</p>
      </div>
      <button class="icon-button" type="button" aria-label="Close API key dialog" onclick={closeCreate}>×</button>
    </header>

    {#if data.dashboardMode === 'demo'}
      <p class="demo-note">Demo mode: preview only. No credential will be created.</p>
    {/if}

    <div class="dialog-body">
      <label class="field">
        <span>Key name</span>
        <input name="name" minlength="2" maxlength="120" required placeholder="Production orders" />
      </label>
      <label class="field">
        <span>Expiry (optional)</span>
        <input name="expires_at" type="datetime-local" />
      </label>
      <fieldset>
        <legend>Scopes</legend>
        <div class="scope-grid">
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

      {#if createResultVisible && !mutationPending && form?.mutation === 'createApiKey' && form?.error}
        <p class="form-message error" role="alert">{form.error.message}</p>
      {/if}

      {#if createResultVisible && !mutationPending && form?.mutation === 'createApiKey' && form?.apiKey}
        <section class="secret-result" aria-live="polite">
          <div>
            <strong>Secret key · shown once</strong>
            <span>{form.apiKey.name}</span>
          </div>
          <code>{form.apiKey.secret}</code>
          <button class="button" type="button" onclick={() => copySecret(form.apiKey.secret)}>
            <Icon name="copy" size={12} /> {copied === form.apiKey.secret ? 'Copied' : 'Copy key'}
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
      >{mutationPending ? 'Creating…' : 'Create secret key'}</button>
    </footer>
  </form>
</dialog>

<dialog bind:this={revokeDialog} aria-labelledby="revoke-api-key-title" onclose={resetRevokeSession}>
  <form
    method="POST"
    action="?/revokeApiKey"
    use:enhance={() => {
      mutationPending = true;
      revokeAttemptVisible = true;
      return async ({ result, update }) => {
        await update();
        mutationPending = false;
        if (result.type === 'success') closeRevoke();
      };
    }}
  >
    <header class="dialog-header">
      <div>
        <h2 id="revoke-api-key-title">Revoke this API key?</h2>
        <p>Requests using this credential will stop authenticating immediately.</p>
      </div>
      <button class="icon-button" type="button" aria-label="Close revoke dialog" onclick={closeRevoke}>×</button>
    </header>
    <div class="dialog-body">
      <input type="hidden" name="api_key_id" value={selectedApiKey?.id ?? ''} />
      <p class="confirm-copy">
        Revoke <strong>{selectedApiKey?.name}</strong>
        (<code>{selectedApiKey?.prefix}••••••</code>)? This cannot be undone.
      </p>
      {#if data.dashboardMode === 'demo'}
        <p class="demo-note inline">Demo mode: no credential will be revoked.</p>
      {/if}
      {#if revokeAttemptVisible && !mutationPending && form?.mutation === 'revokeApiKey' && form?.error}
        <p class="form-message error" role="alert">{form.error.message}</p>
      {/if}
    </div>
    <footer class="dialog-footer">
      <button class="button" type="button" onclick={closeRevoke}>Keep key</button>
      <button
        class="button danger-button"
        type="submit"
        disabled={mutationPending || data.dashboardMode !== 'live'}
      >{mutationPending ? 'Revoking…' : 'Revoke key'}</button>
    </footer>
  </form>
</dialog>

<style>
  .security-note {
    min-height: 57px;
    display: grid;
    grid-template-columns: 30px 1fr auto;
    align-items: center;
    gap: 10px;
    margin: 13px 0;
    padding: 9px 12px;
    color: var(--info);
    background: var(--info-soft);
    border: 1px solid color-mix(in oklch, var(--info), transparent 75%);
    border-radius: var(--radius-md);
  }

  .security-note strong {
    color: var(--text-primary);
    font-size: 10px;
    font-weight: 540;
  }

  .security-note p {
    margin: 2px 0 0;
    color: var(--text-secondary);
    font-size: 9px;
  }

  .security-note a {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 9px;
  }

  .panel {
    overflow-x: auto;
  }

  table {
    width: 100%;
    min-width: 780px;
    border-collapse: collapse;
    font-size: 10px;
  }

  th {
    height: 31px;
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
    height: 49px;
    padding: 0 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  tr:last-child td {
    border-bottom: 0;
  }

  td strong {
    font-weight: 500;
  }

  .copy {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 6px;
    color: var(--text-secondary);
    background: var(--canvas);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .copy code {
    font: 9px var(--font-mono);
  }

  .copy :global(svg) {
    color: var(--text-tertiary);
  }

  .environment,
  .scope-count {
    padding: 2px 6px;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    font-size: 8px;
    text-transform: capitalize;
  }

  .environment.live {
    color: var(--success);
    background: var(--success-soft);
  }

  .action {
    width: 38px;
    padding: 0 6px;
  }

  .action button {
    width: 25px;
    height: 25px;
    display: grid;
    place-items: center;
    color: var(--text-tertiary);
    background: transparent;
    border: 0;
    border-radius: var(--radius-sm);
  }

  .action button:hover {
    color: var(--danger);
    background: var(--danger-soft);
  }

  .quickstart {
    display: grid;
    grid-template-columns: minmax(210px, 0.55fr) minmax(360px, 1fr) auto;
    align-items: center;
    gap: 20px;
    margin-top: 12px;
    padding: 16px;
  }

  .step {
    color: var(--accent);
    font-size: 8px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  h2 {
    margin: 3px 0 0;
    font-size: 12px;
    font-weight: 550;
  }

  .quickstart p {
    margin: 3px 0 0;
    color: var(--text-tertiary);
    font-size: 9px;
    line-height: 14px;
  }

  pre {
    overflow-x: auto;
    margin: 0;
    padding: 9px 11px;
    color: var(--text-secondary);
    background: var(--canvas);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    font: 9px/15px var(--font-mono);
  }

  pre span {
    color: var(--accent-hover);
  }

  pre em {
    color: var(--success);
    font-style: normal;
  }

  dialog {
    width: min(470px, calc(100vw - 24px));
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

  .scope-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 7px 12px;
  }

  .scope-grid label {
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

  .secret-result {
    display: grid;
    gap: 8px;
    padding: 10px;
    background: var(--success-soft);
    border: 1px solid color-mix(in oklch, var(--success), transparent 72%);
    border-radius: var(--radius-md);
  }

  .secret-result > div {
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }

  .secret-result strong {
    color: var(--success);
    font-size: 9px;
    font-weight: 550;
  }

  .secret-result span {
    color: var(--text-tertiary);
    font-size: 8px;
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

  @media (max-width: 960px) {
    .quickstart {
      grid-template-columns: 1fr;
    }

    .quickstart .button {
      justify-self: start;
    }
  }

  @media (max-width: 620px) {
    .security-note {
      grid-template-columns: 30px 1fr;
    }

    .security-note a {
      display: none;
    }
  }
</style>

<script lang="ts">
  import { enhance } from '$app/forms';
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  let { data, form } = $props();
  const nodes = $derived(data.nodes);

  let query = $state('');
  let enrolmentDialog: HTMLDialogElement;
  let enrolmentPending = $state(false);
  let enrolmentAttemptSubmitted = $state(false);
  let enrolmentSessionDismissed = $state(false);
  const enrolmentResultVisible = $derived(
    enrolmentAttemptSubmitted ||
      (!enrolmentSessionDismissed && form?.mutation === 'createEnrolment')
  );
  let copied = $state(false);
  const visible = $derived(
    nodes.filter(
      (node) =>
        query === '' ||
        node.name.toLowerCase().includes(query.toLowerCase()) ||
        node.labels.some((label) => label.includes(query.toLowerCase()))
    )
  );

  async function copyToken(token: string) {
    await navigator.clipboard.writeText(token);
    copied = true;
  }

  function openEnrolment() {
    copied = false;
    enrolmentDialog.showModal();
  }

  function resetEnrolmentSession() {
    enrolmentAttemptSubmitted = false;
    enrolmentSessionDismissed = true;
    copied = false;
  }

  function closeEnrolment() {
    resetEnrolmentSession();
    enrolmentDialog.close();
  }
</script>

<svelte:head><title>Nodes · Spool</title></svelte:head>

{#snippet actions()}
  <a class="button" href="/dashboard/local"><Icon name="printers" size={13} /> This device</a>
  <a class="button" href="/downloads"><Icon name="docs" size={13} /> Downloads</a>
  <button class="button primary" onclick={openEnrolment}><Icon name="plus" size={13} /> Add node</button>
{/snippet}

<PageHeader
  title="Nodes"
  description="Small services that discover printers, preserve local queues, and replay native driver profiles."
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

<div class="toolbar">
  <label class="search">
    <Icon name="search" size={13} />
    <input bind:value={query} aria-label="Search nodes" placeholder="Search nodes…" />
  </label>
  <span class="count numeric">{visible.length} nodes</span>
</div>

<section class="agent-grid">
  {#each visible as node}
    <article class="panel">
      <header>
        <span class="os-icon"><Icon name="agents" size={15} /></span>
        <div class="title">
          <strong>{node.name}</strong>
          <span class="mono">{node.id}</span>
        </div>
        <a class="agent-details" aria-label={`View ${node.name}`} href={`/dashboard/nodes/${node.id}`}><Icon name="arrow-right" size={13} /></a>
      </header>
      <div class="health">
        <Status value={node.state} />
        <span>Seen <RelativeTime value={node.lastSeenAt} /></span>
      </div>
      <dl>
        <div><dt>Platform</dt><dd>{node.os} · {node.architecture}</dd></div>
        <div><dt>Node version</dt><dd class="mono">v{node.version}</dd></div>
        <div><dt>Printers</dt><dd class="numeric">{node.printerCount}</dd></div>
        <div><dt>Local queue</dt><dd class="numeric">{node.queueDepth} jobs</dd></div>
      </dl>
      <footer>
        <div class="labels">
          {#each node.labels as label}<span>{label}</span>{/each}
        </div>
        <a href={`/dashboard/nodes/${node.id}`}>Details <Icon name="arrow-right" size={11} /></a>
      </footer>
    </article>
  {/each}
</section>

<dialog bind:this={enrolmentDialog} aria-labelledby="enrolment-title" onclose={resetEnrolmentSession}>
  <form
    method="POST"
    action="?/createEnrolment"
    use:enhance={() => {
      enrolmentPending = true;
      enrolmentAttemptSubmitted = true;
      enrolmentSessionDismissed = false;
      copied = false;
      return async ({ update }) => {
        await update({ reset: false });
        enrolmentPending = false;
      };
    }}
  >
    <header class="dialog-header">
      <div>
        <h2 id="enrolment-title">Add a node</h2>
        <p>Install the native app, then approve its short-lived browser pairing request.</p>
      </div>
      <button
        class="icon-button"
        type="button"
        aria-label="Close enrolment dialog"
        onclick={closeEnrolment}
      >×</button>
    </header>

    {#if data.dashboardMode === 'demo'}
      <p class="demo-note">Demo mode: preview only. No enrolment will be created.</p>
    {/if}

    <div class="dialog-body">
      <ol class="onboarding-steps" aria-label="Add node steps">
        <li><span>1</span><div><strong>Install</strong><small><a href="/downloads">Download the native node</a> on the printer computer.</small></div></li>
        <li><span>2</span><div><strong>Connect node</strong><small>Choose Connect node in the native tray or menu app.</small></div></li>
        <li><span>3</span><div><strong>Approve</strong><small>Match the computer and one-time code in the browser, then approve it.</small></div></li>
      </ol>

      <section class="browser-pairing">
        <span><Icon name="check" size={13} /></span>
        <div>
          <strong>Browser pairing is recommended</strong>
          <small>The device key stays on the printer computer. The browser approves only its public identity.</small>
        </div>
        <a class="button" href="/pair">Pairing instructions</a>
      </section>

      <div class="manual-heading">
        <strong>Manual token fallback</strong>
        <span>Use only when the native app cannot open the browser pairing flow.</span>
      </div>
      <label>
        <span>Node name</span>
        <input name="name" minlength="2" maxlength="120" required placeholder="Warehouse Mac mini" />
      </label>
      <label>
        <span>Token expiry</span>
        <select name="expires_in_seconds">
          <option value="600">10 minutes</option>
          <option value="1800">30 minutes</option>
          <option value="3600">1 hour</option>
        </select>
      </label>

      {#if enrolmentResultVisible && !enrolmentPending && form?.mutation === 'createEnrolment' && form?.error}
        <p class="form-message error" role="alert">{form.error.message}</p>
      {/if}

      {#if enrolmentResultVisible && !enrolmentPending && form?.mutation === 'createEnrolment' && form?.enrolment}
        <section class="secret-result" aria-live="polite">
          <div>
            <strong>Node token · shown once</strong>
            <span>Expires {new Date(form.enrolment.expiresAt).toLocaleString()}</span>
          </div>
          <code>{form.enrolment.token}</code>
          <button
            class="button"
            type="button"
            onclick={() => copyToken(form.enrolment.token)}
          ><Icon name="copy" size={12} /> {copied ? 'Copied' : 'Copy token'}</button>
        </section>
      {/if}
    </div>

    <footer class="dialog-footer">
      <button class="button" type="button" onclick={closeEnrolment}>Close</button>
      <button
        class="button"
        type="submit"
        disabled={enrolmentPending || data.dashboardMode !== 'live'}
      >{enrolmentPending ? 'Creating…' : 'Create manual token'}</button>
    </footer>
  </form>
</dialog>

<style>
  .toolbar {
    min-height: 53px;
    display: flex;
    align-items: center;
  }

  .search {
    width: 220px;
    height: 29px;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 8px;
    color: var(--text-tertiary);
    background: var(--surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
  }

  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  input {
    min-width: 0;
    width: 100%;
    color: var(--text-primary);
    background: transparent;
    border: 0;
    outline: 0;
    font-size: 11px;
  }

  .count {
    margin-left: auto;
    color: var(--text-tertiary);
    font-size: 10px;
  }

  .agent-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(260px, 1fr));
    gap: 10px;
  }

  article {
    overflow: hidden;
  }

  article > header {
    height: 54px;
    display: grid;
    grid-template-columns: 31px 1fr 26px;
    align-items: center;
    gap: 9px;
    padding: 0 11px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .os-icon {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: 8px;
  }

  .title {
    min-width: 0;
    display: grid;
    line-height: 15px;
  }

  .title strong {
    overflow: hidden;
    font-size: 11px;
    font-weight: 540;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .title span {
    color: var(--text-tertiary);
    font-size: 8px;
  }

  .agent-details {
    width: 25px;
    height: 25px;
    display: grid;
    place-items: center;
    color: var(--text-tertiary);
    border-radius: var(--radius-sm);
  }

  .agent-details:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .health {
    height: 35px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .health > span {
    color: var(--text-tertiary);
    font-size: 9px;
  }

  dl {
    margin: 0;
    padding: 7px 12px;
  }

  dl div {
    height: 27px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  dt {
    color: var(--text-tertiary);
    font-size: 10px;
  }

  dd {
    margin: 0;
    color: var(--text-secondary);
    font-size: 10px;
    text-transform: capitalize;
  }

  footer {
    min-height: 39px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 6px 11px;
    background: color-mix(in oklch, var(--canvas), transparent 35%);
    border-top: 1px solid var(--border-subtle);
  }

  .labels {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .labels span {
    padding: 2px 5px;
    color: var(--text-tertiary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    font-size: 8px;
  }

  footer a {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--text-secondary);
    font-size: 9px;
    white-space: nowrap;
  }

  footer a:hover {
    color: var(--text-primary);
  }

  dialog {
    width: min(440px, calc(100vw - 24px));
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

  .dialog-body {
    display: grid;
    gap: 12px;
    padding: 14px;
  }

  .onboarding-steps {
    display: grid;
    gap: 5px;
    margin: 0;
    padding: 0 0 3px;
    list-style: none;
  }

  .onboarding-steps li {
    display: grid;
    grid-template-columns: 21px 1fr;
    align-items: center;
    gap: 8px;
  }

  .onboarding-steps li > span {
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    color: var(--text-tertiary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: 50%;
    font-size: 8px;
  }

  .onboarding-steps div {
    display: grid;
  }

  .onboarding-steps strong {
    font-size: 9px;
    font-weight: 540;
  }

  .onboarding-steps small {
    color: var(--text-tertiary);
    font-size: 8px;
    line-height: 13px;
  }

  .onboarding-steps a {
    color: var(--text-secondary);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .browser-pairing {
    display: grid;
    grid-template-columns: 27px 1fr auto;
    align-items: center;
    gap: 9px;
    padding: 9px;
    background: var(--success-soft);
    border: 1px solid color-mix(in oklch, var(--success), transparent 76%);
    border-radius: var(--radius-md);
  }

  .browser-pairing > span {
    width: 25px;
    height: 25px;
    display: grid;
    place-items: center;
    color: var(--success);
    background: color-mix(in oklch, var(--success), transparent 88%);
    border-radius: 6px;
  }

  .browser-pairing > div,
  .manual-heading {
    display: grid;
    gap: 2px;
  }

  .browser-pairing strong,
  .manual-heading strong {
    font-size: 9px;
    font-weight: 560;
  }

  .browser-pairing small,
  .manual-heading span {
    color: var(--text-tertiary);
    font-size: 8px;
    line-height: 12px;
  }

  .manual-heading {
    padding-top: 2px;
  }

  .dialog-body > label {
    display: grid;
    gap: 5px;
    color: var(--text-secondary);
    font-size: 9px;
  }

  .dialog-body input,
  .dialog-body select {
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

  .dialog-body input:focus,
  .dialog-body select:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  .form-message {
    margin: 0;
    padding: 7px 8px;
    border-radius: var(--radius-md);
    font-size: 9px;
  }

  .form-message.error {
    color: var(--danger);
    background: var(--danger-soft);
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

  @media (max-width: 1100px) {
    .agent-grid {
      grid-template-columns: repeat(2, minmax(260px, 1fr));
    }
  }

  @media (max-width: 620px) {
    .agent-grid {
      grid-template-columns: 1fr;
    }

    .search {
      width: 100%;
    }

    .count {
      display: none;
    }
  }
</style>

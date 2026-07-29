<script lang="ts">
  import { enhance } from '$app/forms';
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';

  let { data, form } = $props();
  const job = $derived(data.job);
  const printer = $derived(data.printer);
  const agent = $derived(data.agent);
  const jobEvents = $derived(data.jobEvents);
  const cancellationAvailable = $derived(
    job !== null &&
      !['completed_reported', 'cancelled', 'expired', 'failed_terminal'].includes(job.state)
  );
  let cancelDialog = $state<HTMLDialogElement>();
  let cancellationPending = $state(false);
  let cancellationAttemptVisible = $state(false);

  function openCancellation() {
    cancellationAttemptVisible = false;
    cancelDialog?.showModal();
  }

  function resetCancellationSession() {
    cancellationAttemptVisible = false;
  }

  function closeCancellation() {
    resetCancellationSession();
    cancelDialog?.close();
  }
</script>

<svelte:head><title>{job?.title ?? 'Job unavailable'} · Piqae</title></svelte:head>

{#if data.dataError}
  <PageHeader eyebrow="Print job" title="Job unavailable" description={data.dataError.code} />
  <DataError error={data.dataError} />
{:else if job}
{#snippet actions()}
  <button
    class="button"
    disabled={!cancellationAvailable}
    title={cancellationAvailable ? 'Request job cancellation' : 'This job is already terminal'}
    onclick={openCancellation}
  >Cancel</button>
  <button class="button" disabled title="Reprint mutation is not implemented"><Icon name="copy" size={13} /> Reprint</button>
{/snippet}

<PageHeader eyebrow="Print job" title={job.title} description={job.id} {actions} />

<div class="status-line">
  <Status value={job.state} />
  <span class="separator">·</span>
  <span>{job.message}</span>
  <span class="spacer"></span>
  <span class="muted">Updated <RelativeTime value={job.updatedAt} /></span>
</div>

{#if job.state === 'delivery_uncertain'}
  <section class="uncertain">
    <span class="uncertain-icon"><Icon name="warning" size={16} /></span>
    <div>
      <strong>Piqae cannot safely determine whether this job printed</strong>
      <p>
        The node restarted between the OS handoff and recording its native job ID. Automatic retry
        is disabled to prevent a duplicate.
      </p>
    </div>
    <button class="button" disabled title="Uncertain-delivery resolution is not implemented">Resolve</button>
  </section>
{/if}

<div class="detail-grid">
  <section class="panel timeline">
    <header><h2>Event timeline</h2><span>{jobEvents.length} events</span></header>
    <ol>
      {#each jobEvents.slice().reverse() as event, index}
        <li>
          <span class:current={index === 0} class="marker"><span></span></span>
          <div class="event">
            <div class="event-head">
              <strong>{event.message}</strong>
              <time class="numeric" datetime={event.occurredAt}>
                {new Date(event.occurredAt).toLocaleTimeString([], {
                  hour: '2-digit',
                  minute: '2-digit',
                  second: '2-digit'
                })}
              </time>
            </div>
            <p>
              <span>{event.observer.replaceAll('_', ' ')}</span>
              <span>·</span>
              <span>{event.authority.replaceAll('_', ' ')}</span>
              <span>·</span>
              <span class="mono">sequence {event.sequence}</span>
            </p>
            {#if Object.keys(event.details).length > 0}
              <pre>{JSON.stringify(event.details, null, 2)}</pre>
            {/if}
          </div>
        </li>
      {/each}
    </ol>
  </section>

  <aside class="properties">
    <section class="panel property-panel">
      <header><h2>Job details</h2></header>
      <dl>
        <div><dt>Status</dt><dd><Status value={job.state} /></dd></div>
        <div><dt>Printer</dt><dd><a href={`/dashboard/printers/${job.printerId}`}>{printer?.name}</a></dd></div>
        <div><dt>Node</dt><dd><a href="/dashboard/nodes">{agent?.name}</a></dd></div>
        <div><dt>Format</dt><dd>{job.contentFormat.toUpperCase()}</dd></div>
        <div><dt>Source</dt><dd>{job.source ?? '—'}</dd></div>
        <div><dt>Authority</dt><dd>{job.authority.replaceAll('_', ' ')}</dd></div>
        <div><dt>Native job</dt><dd class="mono">{job.nativeJobId ?? '—'}</dd></div>
        <div><dt>Content</dt><dd>{job.contentRetained ? 'Retained' : 'Deleted'}</dd></div>
      </dl>
    </section>

    <section class="panel property-panel">
      <header><h2>Identifiers</h2></header>
      <dl>
        <div><dt>Job</dt><dd class="mono">{job.id}</dd></div>
        <div><dt>Printer</dt><dd class="mono">{job.printerId}</dd></div>
        <div><dt>Node</dt><dd class="mono">{job.agentId}</dd></div>
      </dl>
    </section>
  </aside>
</div>

<dialog bind:this={cancelDialog} aria-labelledby="cancel-job-title" onclose={resetCancellationSession}>
  <form
    method="POST"
    action="?/cancel"
    use:enhance={() => {
      cancellationPending = true;
      cancellationAttemptVisible = true;
      return async ({ result, update }) => {
        await update();
        cancellationPending = false;
        if (result.type === 'success') closeCancellation();
      };
    }}
  >
    <header class="dialog-header">
      <div>
        <h2 id="cancel-job-title">Cancel this print job?</h2>
        <p>Piqae will send a cancellation request to the node’s durable local queue.</p>
      </div>
      <button class="icon-button" type="button" aria-label="Close cancellation dialog" onclick={closeCancellation}>×</button>
    </header>
    <div class="dialog-body">
      <p class="confirm-copy">
        Cancel <strong>{job.title}</strong>? If the operating system has already handed the document
        to the printer, cancellation may not stop physical output. Piqae will not create an
        automatic duplicate.
      </p>
      {#if data.dashboardMode === 'demo'}
        <p class="demo-note">Demo mode: no cancellation request will be sent.</p>
      {/if}
      {#if cancellationAttemptVisible && !cancellationPending && form?.mutation === 'cancel' && form?.error}
        <p class="form-message error" role="alert">{form.error.message}</p>
      {/if}
    </div>
    <footer class="dialog-footer">
      <button class="button" type="button" onclick={closeCancellation}>Keep printing</button>
      <button
        class="button danger-button"
        type="submit"
        disabled={cancellationPending || data.dashboardMode !== 'live'}
      >{cancellationPending ? 'Cancelling…' : 'Confirm cancellation'}</button>
    </footer>
  </form>
</dialog>

<style>
  .status-line {
    min-height: 47px;
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-secondary);
    font-size: 11px;
  }

  .separator {
    color: var(--text-tertiary);
  }

  .spacer {
    flex: 1;
  }

  .uncertain {
    min-height: 75px;
    display: grid;
    grid-template-columns: 32px 1fr auto;
    align-items: center;
    gap: 12px;
    margin: 1px 0 13px;
    padding: 12px 14px;
    background: var(--danger-soft);
    border: 1px solid color-mix(in oklch, var(--danger), transparent 72%);
    border-radius: var(--radius-lg);
  }

  .uncertain-icon {
    width: 31px;
    height: 31px;
    display: grid;
    place-items: center;
    color: var(--danger);
    background: color-mix(in oklch, var(--danger), transparent 85%);
    border-radius: 8px;
  }

  .uncertain strong {
    font-size: 11px;
    font-weight: 550;
  }

  .uncertain p {
    margin: 3px 0 0;
    color: var(--text-secondary);
    font-size: 10px;
    line-height: 15px;
  }

  .detail-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 310px;
    align-items: start;
    gap: 12px;
  }

  .timeline > header,
  .property-panel > header {
    height: 43px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 13px;
    border-bottom: 1px solid var(--border-subtle);
  }

  h2 {
    margin: 0;
    font-size: 11px;
    font-weight: 550;
  }

  header > span {
    color: var(--text-tertiary);
    font-size: 9px;
  }

  ol {
    margin: 0;
    padding: 7px 0;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: 38px minmax(0, 1fr);
    min-height: 72px;
  }

  .marker {
    position: relative;
    display: flex;
    justify-content: center;
  }

  .marker::after {
    position: absolute;
    inset: 27px auto 0;
    width: 1px;
    content: '';
    background: var(--border-default);
  }

  li:last-child .marker::after {
    display: none;
  }

  .marker span {
    position: relative;
    z-index: 1;
    width: 8px;
    height: 8px;
    margin-top: 17px;
    background: var(--surface-raised);
    border: 2px solid var(--text-tertiary);
    border-radius: 50%;
  }

  .marker.current span {
    background: var(--accent);
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }

  .event {
    padding: 13px 14px 14px 0;
    border-bottom: 1px solid var(--border-subtle);
  }

  li:last-child .event {
    border-bottom: 0;
  }

  .event-head {
    display: flex;
    justify-content: space-between;
    gap: 16px;
  }

  .event-head strong {
    font-size: 11px;
    font-weight: 500;
  }

  .event-head time {
    color: var(--text-tertiary);
    font-size: 9px;
  }

  .event p {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    margin: 3px 0 0;
    color: var(--text-tertiary);
    font-size: 9px;
    text-transform: capitalize;
  }

  .event pre {
    overflow-x: auto;
    margin: 8px 0 0;
    padding: 8px;
    color: var(--text-secondary);
    background: var(--canvas);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    font: 9px/14px var(--font-mono);
  }

  .properties {
    display: grid;
    gap: 12px;
  }

  dl {
    margin: 0;
    padding: 6px 13px;
  }

  dl div {
    min-height: 32px;
    display: grid;
    grid-template-columns: 88px minmax(0, 1fr);
    align-items: center;
    border-bottom: 1px solid var(--border-subtle);
  }

  dl div:last-child {
    border-bottom: 0;
  }

  dt {
    color: var(--text-tertiary);
    font-size: 10px;
  }

  dd {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--text-secondary);
    font-size: 10px;
    text-align: right;
    text-overflow: ellipsis;
    text-transform: capitalize;
    white-space: nowrap;
  }

  dd a:hover {
    color: var(--text-primary);
  }

  dialog {
    width: min(450px, calc(100vw - 24px));
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

  .dialog-header p {
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

  .dialog-body {
    display: grid;
    gap: 10px;
    padding: 14px;
  }

  .confirm-copy,
  .demo-note,
  .form-message {
    margin: 0;
    padding: 8px;
    border-radius: var(--radius-md);
    font-size: 9px;
    line-height: 15px;
  }

  .confirm-copy {
    color: var(--text-secondary);
    background: var(--canvas);
    border: 1px solid var(--border-subtle);
  }

  .demo-note {
    color: var(--warning);
    background: var(--warning-soft);
  }

  .form-message.error {
    color: var(--danger);
    background: var(--danger-soft);
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

  @media (max-width: 900px) {
    .detail-grid {
      grid-template-columns: 1fr;
    }

    .properties {
      grid-template-columns: 1fr 1fr;
    }
  }

  @media (max-width: 620px) {
    .status-line {
      align-items: flex-start;
      flex-wrap: wrap;
      padding: 9px 0;
    }

    .spacer {
      display: none;
    }

    .uncertain {
      grid-template-columns: 32px 1fr;
    }

    .uncertain .button {
      grid-column: 2;
      justify-self: start;
    }

    .properties {
      grid-template-columns: 1fr;
    }
  }
</style>
{/if}

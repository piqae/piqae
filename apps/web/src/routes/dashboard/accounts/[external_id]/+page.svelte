<script lang="ts">
  import DataError from '$lib/components/DataError.svelte';
  import Icon from '$lib/components/Icon.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';

  let { data } = $props();
  let copied = $state<string | null>(null);

  const account = $derived(data.account);
  const retrieveSnippet = $derived(
    account
      ? `import { SpoolPlatform } from '@spool/sdk';

const spool = new SpoolPlatform({
  platformKey: process.env.SPOOL_PLATFORM_KEY!
});
const account = await spool.accounts.retrieve(${JSON.stringify(account.externalId)});`
      : ''
  );
  const printSnippet = $derived(
    account
      ? `import { readFile } from 'node:fs/promises';
import { SpoolPlatform } from '@spool/sdk';

const spool = new SpoolPlatform({
  platformKey: process.env.SPOOL_PLATFORM_KEY!
});
const account = await spool.accounts.retrieve(${JSON.stringify(account.externalId)});
const pdf = await readFile('./packing-label.pdf');
const job = await account.printPdf({
  printerId: 'printer_id',
  title: 'Packing label',
  pdf,
  idempotencyKey: 'order_123-label-v1'
});`
      : ''
  );

  async function copy(value: string, label: string) {
    try {
      await navigator.clipboard.writeText(value);
      copied = label;
    } catch {
      copied = null;
    }
  }
</script>

<svelte:head><title>{account?.name ?? 'Customer'} · Spool</title></svelte:head>

<a class="back" href="/dashboard/accounts"><span aria-hidden="true">←</span> Customers</a>

{#if !data.available}
  <PageHeader
    title="Customer accounts unavailable"
    description="This deployment does not expose the optional platform accounts capability."
  />
{:else if data.dataError}
  <PageHeader title="Customer unavailable" description="Spool could not load this customer." />
  <DataError error={data.dataError} />
{:else if !account}
  <PageHeader title="Customer not found" description="No customer matches this external ID." />
  <section class="panel empty-state">Return to Customers and choose an available account.</section>
{:else}
  {#snippet actions()}
    <Status
      value={account.status}
      label={account.status === 'cancelled' ? 'Archived' : undefined}
    />
  {/snippet}

  <PageHeader
    eyebrow="Customer"
    title={account.name}
    description={account.externalId}
    {actions}
  />

  <section class="summary-grid" aria-label="Customer summary">
    <article class="panel">
      <span>Status</span>
      <strong>
        {account.status === 'cancelled'
          ? 'Archived'
          : account.status === 'suspended'
            ? 'Suspended'
            : 'Active'}
      </strong>
    </article>
    <article class="panel">
      <span>Environments</span>
      <strong>Test and Live</strong>
    </article>
    <article class="panel">
      <span>Last updated</span>
      <strong><RelativeTime value={account.updatedAt} /></strong>
    </article>
  </section>

  <div class="content-grid">
    <section class="panel section">
      <header>
        <div>
          <h2>Server integration</h2>
          <p>Use account-scoped clients only in your trusted backend.</p>
        </div>
      </header>

      <div class="snippet">
        <div>
          <strong>Retrieve this customer</strong>
          <button
            class="button small"
            type="button"
            onclick={() => copy(retrieveSnippet, 'retrieve')}
          ><Icon name="copy" size={11} /> {copied === 'retrieve' ? 'Copied' : 'Copy'}</button>
        </div>
        <pre><code>{retrieveSnippet}</code></pre>
      </div>

      <div class="snippet">
        <div>
          <strong>Send a print job</strong>
          <button
            class="button small"
            type="button"
            onclick={() => copy(printSnippet, 'print')}
          ><Icon name="copy" size={11} /> {copied === 'print' ? 'Copied' : 'Copy'}</button>
        </div>
        <pre><code>{printSnippet}</code></pre>
      </div>
      <p class="server-note">
        The account client prints to Live by default. Use <code>account.test.printPdf(…)</code>
        for Test. Keep the platform key in server environment variables; it is never needed in
        browser code.
      </p>
    </section>

    <div class="side">
      <details class="panel">
        <summary>
          <span>
            <strong>Environment IDs</strong>
            <small>Advanced integration details</small>
          </span>
          <Icon name="chevron-down" size={13} />
        </summary>
        <div class="detail-body">
          <div class="identifier">
            <span>Test</span>
            <code>{account.environments.testId}</code>
            <button
              type="button"
              aria-label="Copy Test environment ID"
              onclick={() => copy(account.environments.testId, 'test')}
            ><Icon name="copy" size={11} /></button>
          </div>
          <div class="identifier">
            <span>Live</span>
            <code>{account.environments.liveId}</code>
            <button
              type="button"
              aria-label="Copy Live environment ID"
              onclick={() => copy(account.environments.liveId, 'live')}
            ><Icon name="copy" size={11} /></button>
          </div>
          <div class="identifier">
            <span>Workspace</span>
            <code>{account.id}</code>
            <button
              type="button"
              aria-label="Copy workspace ID"
              onclick={() => copy(account.id, 'workspace')}
            ><Icon name="copy" size={11} /></button>
          </div>
          <span class="copy-state" aria-live="polite">
            {copied === 'test' || copied === 'live' || copied === 'workspace'
              ? 'Identifier copied'
              : ''}
          </span>
        </div>
      </details>

      <details class="panel">
        <summary>
          <span>
            <strong>Metadata</strong>
            <small>{Object.keys(account.metadata).length} fields</small>
          </span>
          <Icon name="chevron-down" size={13} />
        </summary>
        <dl class="detail-body metadata">
          {#each Object.entries(account.metadata) as [key, value]}
            <div><dt>{key}</dt><dd>{value}</dd></div>
          {:else}
            <div class="muted">No metadata has been added.</div>
          {/each}
        </dl>
      </details>
    </div>
  </div>
{/if}

<style>
  .back {
    min-height: 28px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin-bottom: 8px;
    color: var(--text-secondary);
    font-size: 11px;
  }

  .back:hover {
    color: var(--text-primary);
  }

  .summary-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 10px;
    margin: 14px 0;
  }

  .summary-grid article {
    display: grid;
    gap: 4px;
    padding: 13px;
  }

  .summary-grid span {
    color: var(--text-tertiary);
    font-size: 10px;
  }

  .summary-grid strong {
    font-size: 12px;
    font-weight: 550;
  }

  .content-grid {
    display: grid;
    grid-template-columns: minmax(0, 1.45fr) minmax(260px, 0.75fr);
    gap: 12px;
  }

  .section {
    overflow: hidden;
  }

  .section > header {
    padding: 14px 15px 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  h2,
  p {
    margin: 0;
  }

  h2 {
    font-size: 12px;
    font-weight: 560;
  }

  header p,
  .server-note {
    margin-top: 3px;
    color: var(--text-secondary);
    font-size: 10px;
    line-height: 15px;
  }

  .snippet {
    padding: 13px 15px 0;
  }

  .snippet > div {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 7px;
  }

  .snippet strong {
    font-size: 10px;
    font-weight: 550;
  }

  pre {
    min-width: 0;
    overflow-x: auto;
    margin: 0;
    padding: 11px;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    font-size: 10px;
    line-height: 16px;
  }

  .server-note {
    padding: 12px 15px 14px;
  }

  .side {
    display: grid;
    align-content: start;
    gap: 10px;
  }

  details {
    overflow: hidden;
  }

  summary {
    min-height: 51px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 0 13px;
    cursor: pointer;
    list-style: none;
  }

  summary::-webkit-details-marker {
    display: none;
  }

  summary > span {
    display: grid;
    gap: 2px;
  }

  summary strong {
    font-size: 11px;
    font-weight: 550;
  }

  summary small {
    color: var(--text-tertiary);
    font-size: 9px;
  }

  details[open] summary {
    border-bottom: 1px solid var(--border-subtle);
  }

  details[open] summary :global(svg) {
    transform: rotate(180deg);
  }

  .detail-body {
    display: grid;
    gap: 9px;
    padding: 12px 13px;
  }

  .identifier {
    display: grid;
    grid-template-columns: 45px minmax(0, 1fr) 24px;
    align-items: center;
    gap: 7px;
  }

  .identifier > span {
    color: var(--text-tertiary);
    font-size: 9px;
  }

  .identifier code {
    overflow: hidden;
    color: var(--text-secondary);
    font-size: 9px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .identifier button {
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    color: var(--text-tertiary);
    background: transparent;
    border: 0;
    border-radius: 5px;
  }

  .identifier button:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .copy-state {
    min-height: 12px;
    color: var(--success);
    font-size: 9px;
  }

  .metadata div {
    display: grid;
    grid-template-columns: minmax(70px, 0.45fr) 1fr;
    gap: 8px;
    font-size: 10px;
  }

  .metadata dt {
    color: var(--text-tertiary);
  }

  .metadata dd {
    margin: 0;
    color: var(--text-secondary);
    overflow-wrap: anywhere;
  }

  @media (max-width: 780px) {
    .content-grid {
      grid-template-columns: 1fr;
    }
  }

  @media (max-width: 560px) {
    .summary-grid {
      grid-template-columns: 1fr;
    }
  }
</style>

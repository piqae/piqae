<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  import { webhooks } from '$lib/demo-data';
</script>

<svelte:head><title>Webhooks · Spool</title></svelte:head>

{#snippet actions()}
  <button class="button primary"><Icon name="plus" size={13} /> Add endpoint</button>
{/snippet}

<PageHeader
  title="Webhooks"
  description="Signed, durable event delivery with retries and replay."
  {actions}
/>

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
          <strong>{webhook.description}</strong>
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
      <button aria-label={`Actions for ${webhook.description}`}><Icon name="more" size={14} /></button>
    </article>
  {/each}
</div>

<section class="panel attempts">
  <header>
    <div>
      <h2>Recent deliveries</h2>
      <span>Request and response evidence is retained for 30 days</span>
    </div>
    <button class="button small ghost">View all</button>
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

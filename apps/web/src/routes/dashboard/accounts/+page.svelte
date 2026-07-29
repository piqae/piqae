<script lang="ts">
  import DataError from '$lib/components/DataError.svelte';
  import Icon from '$lib/components/Icon.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';

  let { data } = $props();
  let query = $state('');
  const visibleAccounts = $derived(
    data.accounts.filter((account) => {
      const search = query.trim().toLowerCase();
      return (
        search === '' ||
        account.name.toLowerCase().includes(search) ||
        account.externalId.toLowerCase().includes(search)
      );
    })
  );
</script>

<svelte:head><title>Customers · Spool</title></svelte:head>

<PageHeader
  title="Customers"
  description="Customer printing accounts managed by your platform integration."
/>

{#if !data.available}
  <section class="panel empty-state capability-empty">
    <span><Icon name="agents" size={18} /></span>
    <div>
      <strong>Customer accounts are not enabled</strong>
      <p>This deployment does not expose the optional platform accounts capability.</p>
    </div>
  </section>
{:else}
  {#if data.dataError}<DataError error={data.dataError} />{/if}

  <div class="toolbar">
    <label class="search">
      <span class="sr-only">Search customers</span>
      <Icon name="search" size={13} />
      <input bind:value={query} placeholder="Search customers…" />
    </label>
    <span class="count numeric">{visibleAccounts.length} customers</span>
  </div>

  <div class="panel table-panel" aria-busy="false">
    <table>
      <thead>
        <tr>
          <th>Customer</th>
          <th>Status</th>
          <th>Environments</th>
          <th class="right">Updated</th>
          <th><span class="sr-only">Actions</span></th>
        </tr>
      </thead>
      <tbody>
        {#each visibleAccounts as account}
          <tr>
            <td>
              <a class="account" href={`/dashboard/accounts/${encodeURIComponent(account.externalId)}`}>
                <strong>{account.name}</strong>
                <small class="mono">{account.externalId}</small>
              </a>
            </td>
            <td>
              <Status
                value={account.status}
                label={account.status === 'cancelled' ? 'Archived' : undefined}
              />
            </td>
            <td class="muted">Test and Live</td>
            <td class="right muted numeric"><RelativeTime value={account.updatedAt} /></td>
            <td class="action">
              <a
                class="row-details"
                aria-label={`View ${account.name}`}
                href={`/dashboard/accounts/${encodeURIComponent(account.externalId)}`}
              ><Icon name="arrow-right" size={13} /></a>
            </td>
          </tr>
        {:else}
          <tr>
            <td colspan="5">
              <div class="empty-state">
                {query ? 'No customers match this search.' : 'No customer accounts yet.'}
              </div>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .toolbar {
    min-height: 53px;
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .search {
    width: min(260px, 100%);
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

  .search input {
    min-width: 0;
    width: 100%;
    padding: 0;
    color: var(--text-primary);
    background: transparent;
    border: 0;
    outline: 0;
  }

  .count {
    margin-left: auto;
    color: var(--text-tertiary);
    font-size: 11px;
  }

  .table-panel {
    overflow: hidden;
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  th {
    height: 34px;
    padding: 0 13px;
    color: var(--text-tertiary);
    background: var(--surface-raised);
    border-bottom: 1px solid var(--border-subtle);
    font-size: 10px;
    font-weight: 550;
    text-align: left;
  }

  td {
    height: 52px;
    padding: 7px 13px;
    border-bottom: 1px solid var(--border-subtle);
    font-size: 11px;
  }

  tr:last-child td {
    border-bottom: 0;
  }

  tbody tr:hover {
    background: var(--surface-hover);
  }

  .account {
    display: grid;
    gap: 2px;
    color: var(--text-primary);
  }

  .account strong {
    font-size: 12px;
    font-weight: 550;
  }

  .account small {
    color: var(--text-tertiary);
    font-size: 9px;
  }

  .right {
    text-align: right;
  }

  .action {
    width: 34px;
    padding-left: 0;
  }

  .row-details {
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    color: var(--text-tertiary);
    border-radius: 6px;
  }

  .row-details:hover {
    color: var(--text-primary);
    background: var(--surface-raised);
  }

  .capability-empty {
    min-height: 260px;
    margin-top: 16px;
    align-content: center;
    gap: 11px;
  }

  .capability-empty > span {
    width: 38px;
    height: 38px;
    display: grid;
    place-items: center;
    margin: auto;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: 10px;
  }

  .capability-empty strong {
    color: var(--text-primary);
    font-size: 12px;
  }

  .capability-empty p {
    margin: 4px 0 0;
    font-size: 11px;
  }

  @media (max-width: 720px) {
    th:nth-child(3),
    td:nth-child(3) {
      display: none;
    }
  }

  @media (max-width: 520px) {
    th:nth-child(4),
    td:nth-child(4) {
      display: none;
    }
  }
</style>

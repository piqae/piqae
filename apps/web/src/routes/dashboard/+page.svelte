<script lang="ts">
  import { enhance } from '$app/forms';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { onMount } from 'svelte';
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  import JobTimeline from '$lib/components/dashboard/JobTimeline.svelte';
  import { operationalViews, stateFilters } from '$lib/dashboard-navigation';
  import { nativeNodeConnectUrlFromHandoff } from '$lib/node-connect-fragment';
  import {
    summariseUncertainDelivery,
    summariseUncertainDeliveryOverview,
    UNCERTAIN_DELIVERY_STATE
  } from '$lib/uncertain-delivery';
  import {
    DataPanel,
    DefinitionList,
    Dialog,
    Drawer,
    EmptyState,
    Field,
    Metric,
    SearchField,
    SegmentedControl,
    Toolbar
  } from '$lib/components/ui';

  let { data, form } = $props();

  const overview = $derived(data.overview);
  const detail = $derived(data.detail);
  const managedAccount = $derived(data.managedAccount);
  const views = $derived(
    operationalViews({ platform: { accounts: data.meta.platform.accounts && data.platformEnabled } })
  );

  let query = $state('');
  let customerFilter = $state('all');
  const aggregateCustomers = $derived(data.scope === 'customers' && !managedAccount);
  // The state narrowing is addressed, not held: it arrives validated from the
  // loader so a link can put an operator in front of one set of jobs.
  const stateFilter = $derived(data.stateFilter);

  /**
   * A count of uncertain handoffs cannot separate a job that turned uncertain a
   * minute ago from one nobody has resolved for two hours. The age of the
   * oldest is the number worth reading, so the tile leads with it.
   */
  const uncertain = $derived(
    summariseUncertainDeliveryOverview(
      overview.jobs.uncertain,
      overview.jobs.oldestUncertainSince
    )
  );
  const uncertainTitle = $derived(
    uncertain.count === 0
      ? 'No job is waiting on proof that it printed.'
      : `Piqae cannot prove these jobs printed. Longest unresolved: ${
          uncertain.oldestLabel ?? 'unknown'
        }.`
  );

  // The checklist is scaffolding, not chrome: once a first job exists it stops
  // occupying the top of the operational surface for good.
  const setupStep = $derived(
    data.agents.length === 0 ? 1 : data.printers.length === 0 ? 2 : overview.jobs.recent === 0 ? 3 : 4
  );
  const setupComplete = $derived(setupStep === 4);

  const matches = (...fields: (string | null | undefined)[]) =>
    query === '' ||
    fields.some((field) => field?.toLowerCase().includes(query.toLowerCase()));

  const visibleJobs = $derived(
    data.jobs.filter((job) => {
      const matchesState =
        stateFilter === 'all' ||
        (stateFilter === 'active' &&
          !['completed_reported', 'cancelled', 'expired', 'failed_terminal'].includes(job.state)) ||
        (stateFilter === 'failed' && ['failed_terminal', 'failed_retryable'].includes(job.state)) ||
        job.state === stateFilter;
      const matchesCustomer = customerFilter === 'all' || job.customer?.externalId === customerFilter;
      return matches(job.title, job.id, job.customer?.name, job.customer?.externalId) && matchesState && matchesCustomer;
    })
  );

  const visiblePrinters = $derived(
    data.printers.filter(
      (printer) =>
        matches(printer.name, printer.location, printer.description, printer.customer?.name, printer.customer?.externalId) &&
        (customerFilter === 'all' || printer.customer?.externalId === customerFilter) &&
        (stateFilter === 'all' || printer.state === stateFilter)
    )
  );

  const visibleNodes = $derived(
    data.agents.filter(
      (node) =>
        (matches(node.name, node.id, node.customer?.name, node.customer?.externalId) || node.labels.some((label) => matches(label))) &&
        (customerFilter === 'all' || node.customer?.externalId === customerFilter) &&
        (stateFilter === 'all' || node.state === stateFilter)
    )
  );

  const visibleAccounts = $derived(
    data.accounts.filter((account) => matches(account.name, account.externalId))
  );

  const resultCount = $derived(
    data.view === 'jobs'
      ? visibleJobs.length
      : data.view === 'printers'
        ? visiblePrinters.length
        : data.view === 'nodes'
          ? visibleNodes.length
          : visibleAccounts.length
  );

  const stateOptions = $derived(stateFilters(data.view));

  const DETAIL_KEYS = ['job', 'printer', 'node', 'customer'];

  function buildHref(overrides: Record<string, string | null>): string {
    const params = new URLSearchParams(page.url.searchParams);
    for (const [key, value] of Object.entries(overrides)) {
      if (value === null) params.delete(key);
      else params.set(key, value);
    }
    const search = params.toString();
    return search ? `/dashboard?${search}` : '/dashboard';
  }

  const operationalHref = (overrides: Record<string, string | null>) =>
    buildHref({ ...Object.fromEntries(DETAIL_KEYS.map((key) => [key, null])), ...overrides });

  const detailHref = (kind: string, id: string, customerExternalId?: string | null) =>
    operationalHref({
      [kind]: id,
      ...(customerExternalId ? { managed_customer: customerExternalId, scope: null } : {})
    });

  const listHref = $derived(
    buildHref(Object.fromEntries(DETAIL_KEYS.map((key) => [key, null])))
  );

  function switchView(next: string) {
    void goto(
      operationalHref({
        view: next,
        state: null
      }),
      { keepFocus: true, noScroll: true }
    );
  }

  function filterByState(next: string) {
    void goto(operationalHref({ state: next === 'all' ? null : next }), {
      keepFocus: true,
      noScroll: true
    });
  }

  function closeDrawer() {
    void goto(listHref, { keepFocus: true, noScroll: true });
  }

  const nodeName = (agentId: string, customerExternalId?: string | null) =>
    data.agents.find((agent) => agent.id === agentId && agent.customer?.externalId === customerExternalId)?.name ??
    data.agents.find((agent) => agent.id === agentId)?.name ?? 'Unknown';
  const printerName = (printerId: string, customerExternalId?: string | null) =>
    data.printers.find((printer) => printer.id === printerId && printer.customer?.externalId === customerExternalId)?.name ??
    data.printers.find((printer) => printer.id === printerId)?.name ?? 'Unknown';
  const readyProfiles = (printer: (typeof data.printers)[number]) =>
    printer.profiles.filter((profile) => profile.status === 'ready').length;

  // Enrolment dialog.
  let enrolmentOpen = $state(false);
  let enrolmentPending = $state(false);
  let enrolmentAttempted = $state(false);
  const enrolmentResult = $derived(
    enrolmentAttempted && !enrolmentPending && form?.mutation === 'createEnrolment' ? form : null
  );

  function openNativeNodeConnect(connectUrl: string): boolean {
    const nativeUrl = nativeNodeConnectUrlFromHandoff(connectUrl, window.location.origin);
    if (!nativeUrl) return false;
    window.location.assign(nativeUrl);
    return true;
  }

  onMount(() => {
    if (page.url.searchParams.get('connect-node') !== '1') return;
    enrolmentOpen = true;
    const next = new URL(page.url);
    next.searchParams.delete('connect-node');
    void goto(`${next.pathname}${next.search}${next.hash}`, { replaceState: true, noScroll: true });
  });

  // Cancellation dialog.
  let cancelOpen = $state(false);
  let cancelPending = $state(false);
  let cancelAttempted = $state(false);

  // Reuse the summary so the drawer and the tile can never disagree about age.
  const uncertainFor = $derived(
    detail?.kind === 'job' && detail.job.state === UNCERTAIN_DELIVERY_STATE
      ? summariseUncertainDelivery([detail.job]).oldestLabel
      : null
  );

  const cancellable = $derived(
    detail?.kind === 'job' &&
      !['completed_reported', 'cancelled', 'expired', 'failed_terminal'].includes(detail.job.state)
  );

  // Hosted PDF print dialog.
  let printOpen = $state(false);
  let printPending = $state(false);
  let diagnosticsPending = $state(false);
  let printAttempted = $state(false);
  let printPrinterId = $state('');
  let printProfileId = $state('');
  const printablePrinters = $derived(
    data.printers.filter(
      (printer) => printer.state === 'online' && printer.profiles.some((profile) => profile.status === 'ready')
    )
  );
  const selectedPrintPrinter = $derived(
    printablePrinters.find((printer) => printer.id === printPrinterId) ?? null
  );
  const selectedPrintProfiles = $derived(
    selectedPrintPrinter?.profiles.filter((profile) => profile.status === 'ready') ?? []
  );
  const selectedPrintProfile = $derived(
    selectedPrintProfiles.find((profile) => profile.profileId === printProfileId) ?? null
  );
  const printResult = $derived(
    printAttempted && !printPending && form?.mutation === 'createPrintJob' ? form : null
  );

  function openPrintDialog(printerId = '') {
    printAttempted = false;
    printPrinterId = printerId || printablePrinters[0]?.id || '';
    const printer = printablePrinters.find((candidate) => candidate.id === printPrinterId);
    printProfileId =
      printer?.profiles.find((profile) => profile.status === 'ready' && profile.isDefault)?.profileId ??
      printer?.profiles.find((profile) => profile.status === 'ready')?.profileId ??
      '';
    printOpen = true;
  }

  function updatePrintProfiles() {
    const printer = printablePrinters.find((candidate) => candidate.id === printPrinterId);
    printProfileId =
      printer?.profiles.find((profile) => profile.status === 'ready' && profile.isDefault)?.profileId ??
      printer?.profiles.find((profile) => profile.status === 'ready')?.profileId ??
      '';
  }

</script>

<svelte:head>
  <title>Operations · Piqae</title>
  <meta name="description" content="Live print jobs, printers, and nodes in one operational view." />
</svelte:head>

{#snippet actions()}
  {#if data.view === 'nodes' && data.meta.deployment === 'local'}
    <!-- Loopback diagnostics are only reachable from here now that the Nodes
         page is a view; the native shells still deep-link to /dashboard/local. -->
    <a class="button" href="/dashboard/local"><Icon name="printers" size={14} /> This device</a>
  {/if}
  <a class="button" href="/docs/quickstart"><Icon name="docs" size={14} /> Quickstart</a>
  {#if !aggregateCustomers}<button
    class="button"
    disabled={printablePrinters.length === 0}
    title={printablePrinters.length === 0 ? 'Connect an online printer with a ready profile first' : 'Send a PDF to a printer'}
    onclick={() => openPrintDialog()}
  >
    <Icon name="printers" size={14} /> Print PDF
  </button>
  <button
    class="button primary"
    onclick={() => {
      enrolmentAttempted = false;
      enrolmentOpen = true;
    }}
  >
    <Icon name="plus" size={14} /> Add node
  </button>{/if}
{/snippet}

<PageHeader
  title={managedAccount ? `${managedAccount.name} operations` : aggregateCustomers ? 'Customer operations' : 'Operations'}
  description={managedAccount
    ? 'Live resources in this managed customer’s isolated workspace.'
    : aggregateCustomers
      ? 'Live jobs, printers, and nodes across your managed customers.'
      : 'Live jobs, printers, and nodes in your own workspace.'}
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

{#if managedAccount}
  <section class="managed-context" aria-label="Managed customer context">
    <div>
      <strong>Managing {managedAccount.name} on behalf of your workspace</strong>
      <span>
        Live environment · <span class="mono">{managedAccount.externalId}</span>. Nodes, printers,
        jobs and actions below stay isolated to this customer.
      </span>
    </div>
    <a class="button compact" href={operationalHref({ managed_customer: null, view: 'customers', state: null })}>
      Return to customers
    </a>
  </section>

  {#if data.agents.length > 0 && data.printers.length === 0}
    <p class="ui-note warning" role="status">
      {data.agents.length === 1 ? 'One node is' : `${data.agents.length} nodes are`} visible in this
      customer workspace, but no printer inventory has been reported. Open a node to inspect its
      last-seen time and request a redacted diagnostic report.
    </p>
  {/if}
{/if}

{#if data.platformEnabled && !managedAccount}
  <nav class="operations-scope" aria-label="Operations scope">
    <a class:active={aggregateCustomers} aria-current={aggregateCustomers ? 'page' : undefined} href={operationalHref({ scope: null })}>
      Customer operations
    </a>
    {#if data.ownHasResources}
      <a class:active={!aggregateCustomers} aria-current={!aggregateCustomers ? 'page' : undefined} href={operationalHref({ scope: 'own' })}>
        My workspace
      </a>
    {/if}
    <span>{data.accounts.filter((account) => account.status === 'active').length} active customers</span>
  </nav>
{/if}

{#if !setupComplete && !aggregateCustomers}
  <section class="setup" aria-label="Setup progress">
    <div class="setup-intro">
      <strong>Send your first print</strong>
      <span>One node, one printer, then one API request.</span>
    </div>
    <ol>
      <li class:complete={setupStep > 1} class:current={setupStep === 1}>
        <span class="step">{setupStep > 1 ? '✓' : '1'}</span>
        <a href="/downloads">Install a node</a>
      </li>
      <li class:complete={setupStep > 2} class:current={setupStep === 2}>
        <span class="step">{setupStep > 2 ? '✓' : '2'}</span>
        <a href={operationalHref({ view: 'printers', state: null })}>Confirm a printer</a>
      </li>
      <li class:complete={setupStep > 3} class:current={setupStep === 3}>
        <span class="step">{setupStep > 3 ? '✓' : '3'}</span>
        <a href="/docs/quickstart">Submit a PDF</a>
      </li>
    </ol>
  </section>
{/if}

{#snippet uncertainDetail()}
  {#if uncertain.count === 0}
    No uncertain handoffs
  {:else}
    {uncertain.count} uncertain {uncertain.count === 1 ? 'handoff' : 'handoffs'}{#if uncertain.oldestLabel}
      · <strong>oldest {uncertain.oldestLabel}</strong>
    {/if}
  {/if}
{/snippet}

<section class="metrics" aria-label="Printing overview">
  <Metric
    label="Nodes online"
    value={overview.agents.online}
    total={overview.agents.total}
    detail={`${overview.agents.total - overview.agents.online} unavailable`}
    href={operationalHref({ view: 'nodes', state: null })}
  />
  <Metric
    label="Printers available"
    value={overview.printers.online}
    total={overview.printers.total}
    detail={`${overview.printers.attention} need attention`}
    href={operationalHref({ view: 'printers', state: null })}
  />
  <Metric
    label="Recent jobs"
    value={overview.jobs.recent.toLocaleString()}
    detail={`${overview.jobs.active} active in queues`}
    href={operationalHref({ view: 'jobs', state: null })}
  />
  <Metric
    label="Needs review"
    value={overview.jobs.failed + overview.jobs.uncertain}
    detail={uncertainDetail}
    tone={uncertain.count > 0 ? 'attention' : 'neutral'}
    title={uncertainTitle}
    href={operationalHref({
      view: 'jobs',
      state: uncertain.count > 0 ? 'delivery_uncertain' : null
    })}
  />
</section>

<Toolbar meta={`${resultCount} ${data.view}`}>
  <SegmentedControl
    value={data.view}
    label="Switch operational view"
    options={views}
    onchange={switchView}
  />
  <SearchField bind:value={query} label={`Search ${data.view}`} placeholder={`Search ${data.view}…`} />
  {#if aggregateCustomers && data.view !== 'customers'}
    <select class="ui-select" bind:value={customerFilter} aria-label="Filter by customer">
      <option value="all">All customers</option>
      {#each data.accounts.filter((account) => account.status === 'active') as account}
        <option value={account.externalId}>{account.name}</option>
      {/each}
    </select>
  {/if}
  {#if stateOptions.length > 0}
    <select
      class="ui-select"
      value={stateFilter}
      onchange={(event) => filterByState(event.currentTarget.value)}
      aria-label="Filter by state"
    >
      {#each stateOptions as option}
        <option value={option.value}>{option.label}</option>
      {/each}
    </select>
  {/if}
</Toolbar>

<DataPanel minWidth={data.view === 'jobs' ? '860px' : '760px'}>
  {#if data.view === 'jobs'}
    <table class="ui-data-table">
      <thead>
        <tr>
          <th>Job</th>
          {#if aggregateCustomers}<th>Customer</th>{/if}
          <th>Status</th>
          <th>Printer</th>
          <th>Node</th>
          <th class="right">Updated</th>
        </tr>
      </thead>
      <tbody>
        {#each visibleJobs as job}
          <tr>
            <td>
              <a class="cell-stack" href={detailHref('job', job.id, job.customer?.externalId)}>
                <strong>{job.title}</strong>
                <small class="mono">{job.id} · {job.contentFormat.toUpperCase()}</small>
              </a>
            </td>
            {#if aggregateCustomers}<td><a class="customer-link" href={operationalHref({ managed_customer: job.customer?.externalId ?? null, scope: null })}>{job.customer?.name ?? 'Unknown'}</a></td>{/if}
            <td><Status value={job.state} /></td>
            <td>
              <span class="cell-inline">
                <Icon name="printers" size={14} />
                {printerName(job.printerId, job.customer?.externalId)}
              </span>
            </td>
            <td class="muted">{nodeName(job.agentId, job.customer?.externalId)}</td>
            <td class="right muted numeric"><RelativeTime value={job.updatedAt} /></td>
          </tr>
        {:else}
          <tr><td colspan={aggregateCustomers ? 6 : 5}><EmptyState message="No jobs match this view." compact /></td></tr>
        {/each}
      </tbody>
    </table>
  {:else if data.view === 'printers'}
    <table class="ui-data-table">
      <thead>
        <tr>
          <th>Printer</th>
          {#if aggregateCustomers}<th>Customer</th>{/if}
          <th>Status</th>
          <th>Node</th>
          <th>Profiles</th>
          <th>Queue</th>
          <th class="right">Last seen</th>
        </tr>
      </thead>
      <tbody>
        {#each visiblePrinters as printer}
          <tr>
            <td>
              <a class="cell-stack" href={detailHref('printer', printer.id, printer.customer?.externalId)}>
                <strong>{printer.name}</strong>
                <small>{printer.location ?? printer.description ?? 'No location'}</small>
              </a>
            </td>
            {#if aggregateCustomers}<td><a class="customer-link" href={operationalHref({ managed_customer: printer.customer?.externalId ?? null, scope: null })}>{printer.customer?.name ?? 'Unknown'}</a></td>{/if}
            <td><Status value={printer.state} /></td>
            <td class="muted">{nodeName(printer.agentId)}</td>
            <td class="numeric">
              {readyProfiles(printer)}/{printer.profiles.length} ready
            </td>
            <td class="numeric">{printer.queueDepth}</td>
            <td class="right muted numeric"><RelativeTime value={printer.lastSeenAt} /></td>
          </tr>
        {:else}
          <tr><td colspan={aggregateCustomers ? 7 : 6}><EmptyState message="No printers match this view." compact /></td></tr>
        {/each}
      </tbody>
    </table>
  {:else if data.view === 'nodes'}
    <table class="ui-data-table">
      <thead>
        <tr>
          <th>Node</th>
          {#if aggregateCustomers}<th>Customer</th>{/if}
          <th>Status</th>
          <th>Platform</th>
          <th>Printers</th>
          <th>Queue</th>
          <th class="right">Last seen</th>
        </tr>
      </thead>
      <tbody>
        {#each visibleNodes as node}
          <tr>
            <td>
              <a class="cell-stack" href={detailHref('node', node.id, node.customer?.externalId)}>
                <strong>{node.name}</strong>
                <small class="mono">{node.id}</small>
              </a>
            </td>
            {#if aggregateCustomers}<td><a class="customer-link" href={operationalHref({ managed_customer: node.customer?.externalId ?? null, scope: null })}>{node.customer?.name ?? 'Unknown'}</a></td>{/if}
            <td><Status value={node.state} /></td>
            <td class="muted">{node.os} · {node.architecture}</td>
            <td class="numeric">{node.printerCount}</td>
            <td class="numeric">{node.queueDepth}</td>
            <td class="right muted numeric"><RelativeTime value={node.lastSeenAt} /></td>
          </tr>
        {:else}
          <tr><td colspan={aggregateCustomers ? 7 : 6}><EmptyState message="No nodes match this view." compact /></td></tr>
        {/each}
      </tbody>
    </table>
  {:else}
    <table class="ui-data-table">
      <thead>
        <tr>
          <th>Customer</th>
          <th>Status</th>
          <th>External ID</th>
          <th class="right">Created</th>
        </tr>
      </thead>
      <tbody>
        {#each visibleAccounts as account}
          <tr>
            <td>
              <a class="cell-stack" href={detailHref('customer', account.externalId)}>
                <strong>{account.name}</strong>
                <small class="mono">{account.id}</small>
              </a>
            </td>
            <td><Status value={account.status === 'active' ? 'online' : 'paused'} label={account.status} /></td>
            <td class="mono muted">{account.externalId}</td>
            <td class="right muted numeric"><RelativeTime value={account.createdAt} /></td>
          </tr>
        {:else}
          <tr><td colspan="4"><EmptyState message="No customers match this view." compact /></td></tr>
        {/each}
      </tbody>
    </table>
  {/if}
</DataPanel>

<!-- Detail drawer. The selected entity is addressed by query string so links stay shareable. -->
<Drawer
  open={detail !== null}
  labelledBy="detail-title"
  eyebrow={detail?.kind === 'job'
    ? 'Print job'
    : detail?.kind === 'printer'
      ? 'Destination'
      : detail?.kind === 'node'
        ? 'Node'
        : detail?.kind === 'customer'
          ? 'Customer'
          : 'Detail'}
  title={detail?.kind === 'job'
    ? detail.job.title
    : detail?.kind === 'printer'
      ? detail.printer.name
      : detail?.kind === 'node'
        ? detail.node.name
        : detail?.kind === 'customer'
          ? detail.account.name
          : 'Not found'}
  onclose={closeDrawer}
>
  {#snippet actions()}
    {#if detail?.kind === 'job'}
      <button
        class="button compact"
        disabled={!cancellable}
        title={cancellable ? 'Request job cancellation' : 'This job is already terminal'}
        onclick={() => {
          cancelAttempted = false;
          cancelOpen = true;
        }}
      >
        Cancel
      </button>
    {:else if detail?.kind === 'printer'}
      <button
        class="button compact"
        disabled={detail.printer.state !== 'online' || readyProfiles(detail.printer) === 0}
        onclick={() => openPrintDialog(detail.printer.id)}
      >Print PDF</button>
      <a class="button compact" href={detailHref('node', detail.printer.agentId)}>Open node</a>
    {:else if detail?.kind === 'customer'}
      <a
        class="button compact primary"
        href={operationalHref({
          managed_customer: detail.account.externalId,
          customer: null,
          view: 'nodes',
          state: null
        })}
      >Manage customer</a>
    {/if}
  {/snippet}

  {#if detail?.kind === 'job'}
    <div class="drawer-status">
      <Status value={detail.job.state} />
      <span class="muted">{detail.job.message}</span>
    </div>

    {#if detail.job.state === 'delivery_uncertain'}
      <p class="ui-note error">
        Piqae cannot safely determine whether this job printed after it was handed to the operating
        system. Automatic retry is disabled because it could produce a duplicate.{#if uncertainFor}
          Unresolved for {uncertainFor}.
        {/if}
      </p>
    {/if}

    <DefinitionList
      items={[
        { term: 'Printer', value: detail.printer?.name ?? 'Unknown' },
        { term: 'Node', value: detail.agent?.name ?? 'Unknown' },
        { term: 'Format', value: detail.job.contentFormat.toUpperCase() },
        { term: 'Source', value: detail.job.source },
        { term: 'Authority', value: detail.job.authority.replaceAll('_', ' ') },
        { term: 'Native job', value: detail.job.nativeJobId, mono: true },
        { term: 'Content', value: detail.job.contentRetained ? 'Retained' : 'Deleted' },
        { term: 'Job ID', value: detail.job.id, mono: true }
      ]}
    />

    <div class="drawer-section">
      <h3>Event timeline<span>{detail.events.length} events</span></h3>
      <JobTimeline events={detail.events} />
    </div>
  {:else if detail?.kind === 'printer'}
    <div class="drawer-status">
      <Status value={detail.printer.state} />
      <span class="muted">Seen <RelativeTime value={detail.printer.lastSeenAt} /></span>
    </div>

    <DefinitionList
      items={[
        { term: 'Node', value: detail.agent?.name ?? 'Unknown' },
        { term: 'Location', value: detail.printer.location },
        { term: 'Queue depth', value: detail.printer.queueDepth },
        { term: 'Colour', value: detail.printer.capabilities.color ? 'Supported' : 'Mono only' },
        { term: 'Duplex', value: detail.printer.capabilities.duplex ? 'Supported' : 'Single sided' },
        { term: 'Printer ID', value: detail.printer.id, mono: true }
      ]}
    />

    <div class="drawer-section">
      <h3>Native print profiles<span>{detail.printer.profiles.length}</span></h3>
      {#each detail.printer.profiles as profile}
        <div class="profile">
          <div>
            <strong>{profile.name}{profile.isDefault ? ' — Default' : ''}</strong>
            <small>
              Revision {profile.revision} · {profile.nativeKind.replaceAll('_', ' ')}
              {profile.published ? ' · Published' : ' · Local only'}
            </small>
          </div>
          <Status value={profile.status} />
        </div>
      {:else}
        <p class="muted empty-line">
          No print profiles. Open the Piqae tray application on this node and choose Add profile.
        </p>
      {/each}
    </div>

    <div class="drawer-section">
      <h3>Recent jobs<span>{detail.jobs.length}</span></h3>
      {#each detail.jobs.slice(0, 8) as job}
        <a class="mini-row" href={detailHref('job', job.id)}>
          <span>{job.title}</span>
          <Status value={job.state} />
        </a>
      {:else}
        <p class="muted empty-line">No jobs recorded for this printer.</p>
      {/each}
    </div>
  {:else if detail?.kind === 'node'}
    <div class="drawer-status">
      <Status value={detail.node.state} />
      <span class="muted">Seen <RelativeTime value={detail.node.lastSeenAt} /></span>
    </div>

    <DefinitionList
      items={[
        { term: 'Platform', value: `${detail.node.os} · ${detail.node.architecture}` },
        { term: 'Node version', value: `v${detail.node.version}`, mono: true },
        { term: 'Protocol', value: detail.node.protocolVersion, mono: true },
        { term: 'Local queue', value: `${detail.node.queueDepth} jobs` },
        { term: 'Labels', value: detail.node.labels.join(', ') || null },
        { term: 'Node ID', value: detail.node.id, mono: true }
      ]}
    />

    <div class="drawer-section">
      <h3>Printers<span>{detail.printers.length}</span></h3>
      {#each detail.printers as printer}
        <a class="mini-row" href={detailHref('printer', printer.id)}>
          <span>{printer.name}</span>
          <Status value={printer.state} />
        </a>
      {:else}
        <p class="muted empty-line">This node has not reported any printers.</p>
      {/each}
    </div>

    <div class="drawer-section">
      <h3>Diagnostics</h3>
      <p class="muted empty-line">
        Asks this node for a redacted health report: agent version, queue depth,
        local storage integrity and the last error it recorded. No document
        content is collected.
      </p>
      <form
        method="POST"
        action="?/collectNodeDiagnostics"
        use:enhance={() => {
          diagnosticsPending = true;
          return async ({ update }) => {
            await update({ reset: false });
            diagnosticsPending = false;
          };
        }}
      >
        {#if managedAccount}<input type="hidden" name="managed_customer" value={managedAccount.externalId} />{/if}
        <input type="hidden" name="node_id" value={detail.node.id} />
        <button class="button small" type="submit" disabled={diagnosticsPending || data.dashboardMode !== 'live'}>
          {diagnosticsPending ? 'Requesting…' : 'Collect diagnostics'}
        </button>
      </form>
      {#await detail.diagnostics}
        <p class="muted empty-line">Loading reports…</p>
      {:then diagnostics}
        {#if diagnostics.dataError}<DataError error={diagnostics.dataError} />{/if}
        {#each diagnostics.reports.slice(0, 5) as report}
          <div class="mini-row">
            <span class="cell-stack">
              <strong>{report.state}</strong>
              <small class="mono">
                {report.agentVersion ? `v${report.agentVersion}` : report.requestId}
                {report.lastErrorCode ? ` · ${report.lastErrorCode}` : ''}
                {report.storageHealthy === false ? ' · storage damaged' : ''}
              </small>
            </span>
            <small class="muted"><RelativeTime value={report.receivedAt ?? report.requestedAt} /></small>
          </div>
        {:else}
          <p class="muted empty-line">No diagnostic report has been collected yet.</p>
        {/each}
      {/await}
    </div>
  {:else if detail?.kind === 'customer'}
    <DefinitionList
      items={[
        { term: 'Status', value: detail.account.status },
        { term: 'External ID', value: detail.account.externalId, mono: true },
        { term: 'Test environment', value: detail.account.environments.testId, mono: true },
        { term: 'Live environment', value: detail.account.environments.liveId, mono: true }
      ]}
    />
  {:else if detail?.kind === 'missing'}
    <EmptyState message={`That ${detail.label} no longer exists.`} compact />
  {/if}
</Drawer>

<!-- Hosted PDF print -->
<Dialog
  bind:open={printOpen}
  labelledBy="create-print-job-title"
  title="Print a PDF"
  description="Choose the destination and driver profile, then add one PDF document."
>
  <div class="ui-dialog__body">
    <form
      id="create-print-job-form"
      method="POST"
      action="?/createPrintJob"
      enctype="multipart/form-data"
      use:enhance={() => {
        printPending = true;
        printAttempted = true;
        return async ({ result, update }) => {
          await update({ reset: false });
          printPending = false;
          if (result.type === 'success') {
            const actionData = result.data as { createdJobId?: unknown } | undefined;
            if (typeof actionData?.createdJobId === 'string') {
              printOpen = false;
              await goto(detailHref('job', actionData.createdJobId));
            }
          }
        };
      }}
    >
      {#if managedAccount}<input type="hidden" name="managed_customer" value={managedAccount.externalId} />{/if}
      <div class="print-fields">
        <Field label="Printer">
          <select class="ui-select" name="printer_id" bind:value={printPrinterId} onchange={updatePrintProfiles} required>
            {#each printablePrinters as printer}
              <option value={printer.id}>{printer.name} — {nodeName(printer.agentId)}</option>
            {/each}
          </select>
        </Field>

        <Field label="Print profile">
          <select class="ui-select" name="profile_id" bind:value={printProfileId} required>
            {#each selectedPrintProfiles as profile}
              <option value={profile.profileId}>
                {profile.name}{profile.isDefault ? ' — Default' : ''}
              </option>
            {/each}
          </select>
        </Field>

        {#if selectedPrintProfile}
          <p class="profile-summary">
            {[selectedPrintProfile.summary.paper, selectedPrintProfile.summary.color, selectedPrintProfile.summary.resolution]
              .filter(Boolean)
              .join(' · ') || 'Uses the saved native driver settings.'}
          </p>
        {/if}

        <Field label="PDF document">
          <input class="input file-input" name="document" type="file" accept="application/pdf,.pdf" required />
        </Field>

        <div class="print-grid">
          <Field label="Job name (optional)">
            <input class="input" name="title" maxlength="180" placeholder="Uses the PDF filename" />
          </Field>
          <Field label="Copies">
            <input class="input" name="copies" type="number" min="1" max="100" value="1" required />
          </Field>
        </div>
      </div>

      <p class="ui-note neutral">
        Creating the job means it was accepted into the queue—not that paper has printed. Live status will appear in Jobs.
      </p>
    </form>

    {#if data.dashboardMode === 'demo'}
      <p class="ui-note warning">Demo mode: no document will be submitted.</p>
    {/if}
    {#if printResult?.error}
      <p class="ui-note error" role="alert">{printResult.error.message}</p>
    {/if}
  </div>

  {#snippet footer()}
    <button class="button" type="button" onclick={() => (printOpen = false)}>Cancel</button>
    <button
      class="button primary"
      type="submit"
      form="create-print-job-form"
      disabled={printPending || !printPrinterId || !printProfileId || data.dashboardMode !== 'live'}
    >
      {printPending ? 'Creating job…' : 'Add to print queue'}
    </button>
  {/snippet}
</Dialog>

<!-- Node enrolment -->
<Dialog
  bind:open={enrolmentOpen}
  labelledBy="enrolment-title"
  title="Add a node"
  description="Connect this workspace to a printer computer."
>
  <div class="ui-dialog__body">
    {#if data.dashboardMode === 'demo'}
      <p class="ui-note warning">Demo mode: preview only. No enrolment will be created.</p>
    {/if}

    <ol class="connect-steps">
      <li><span>1</span><div><strong>Install Piqae</strong><small><a href="/downloads">Download the native node</a> on the printer computer if needed.</small></div></li>
      <li><span>2</span><div><strong>Open the app</strong><small>Continue below to hand the short-lived invitation directly to Piqae.</small></div></li>
      <li><span>3</span><div><strong>Choose printer access</strong><small>Allow every printer on this computer, including ones added later, or select specific printers.</small></div></li>
    </ol>

    <form
      id="enrolment-form"
      method="POST"
      action="?/createEnrolment"
      use:enhance={() => {
        enrolmentPending = true;
        enrolmentAttempted = true;
        return async ({ result, update }) => {
          await update({ reset: false });
          enrolmentPending = false;
          const actionData = result.type === 'success'
            ? result.data as { enrolment?: { connectUrl?: unknown } } | undefined
            : undefined;
          if (typeof actionData?.enrolment?.connectUrl === 'string') {
            openNativeNodeConnect(actionData.enrolment.connectUrl);
          }
        };
      }}
    >
      {#if managedAccount}<input type="hidden" name="managed_customer" value={managedAccount.externalId} />{/if}
      <details class="advanced-options">
        <summary>Advanced options</summary>
        <Field label="Custom node name (optional)">
          <input class="input" name="name" minlength="2" maxlength="120" placeholder="Warehouse Mac mini" />
        </Field>
        <small>Leave blank to use this computer’s name.</small>
      </details>
      <input type="hidden" name="expires_in_seconds" value="600" />
    </form>

    {#if enrolmentResult?.error}
      <p class="ui-note error" role="alert">{enrolmentResult.error.message}</p>
    {/if}

    {#if enrolmentResult?.enrolment}
      <section class="secret" aria-live="polite">
        <div>
          <strong>Did Piqae not open?</strong>
          <span>Retry the app, or use connection help if it is not installed.</span>
        </div>
        <div class="connection-actions">
          <button class="button compact" type="button" onclick={() => openNativeNodeConnect(enrolmentResult.enrolment.connectUrl)}>
            Open Piqae again
          </button>
          <a class="button compact" href={enrolmentResult.enrolment.connectUrl}>Install or troubleshoot</a>
        </div>
      </section>
    {/if}
  </div>

  {#snippet footer()}
    <button class="button" type="button" onclick={() => (enrolmentOpen = false)}>Close</button>
    <button
      class="button"
      type="submit"
      form="enrolment-form"
      disabled={enrolmentPending || data.dashboardMode !== 'live'}
    >
      {enrolmentPending ? 'Opening Piqae…' : 'Continue in Piqae'}
    </button>
  {/snippet}
</Dialog>

<!-- Job cancellation -->
{#if detail?.kind === 'job'}
  <Dialog
    bind:open={cancelOpen}
    labelledBy="cancel-job-title"
    title="Cancel this print job?"
    description="Piqae will send a cancellation request to the node’s durable local queue."
  >
    <div class="ui-dialog__body">
      <form
        id="cancel-form"
        method="POST"
        action="?/cancelJob"
        use:enhance={() => {
          cancelPending = true;
          cancelAttempted = true;
          return async ({ result, update }) => {
            await update();
            cancelPending = false;
            if (result.type === 'success') cancelOpen = false;
          };
        }}
      >
        {#if managedAccount}<input type="hidden" name="managed_customer" value={managedAccount.externalId} />{/if}
        <input type="hidden" name="job_id" value={detail.job.id} />
        <p class="ui-note neutral">
          Cancel <strong>{detail.job.title}</strong>? If the operating system has already handed the
          document to the printer, cancellation may not stop physical output. Piqae will not create
          an automatic duplicate.
        </p>
      </form>
      {#if data.dashboardMode === 'demo'}
        <p class="ui-note warning">Demo mode: no cancellation request will be sent.</p>
      {/if}
      {#if cancelAttempted && !cancelPending && form?.mutation === 'cancelJob' && form?.error}
        <p class="ui-note error" role="alert">{form.error.message}</p>
      {/if}
    </div>

    {#snippet footer()}
      <button class="button" type="button" onclick={() => (cancelOpen = false)}>Keep printing</button>
      <button
        class="button danger-solid"
        type="submit"
        form="cancel-form"
        disabled={cancelPending || data.dashboardMode !== 'live'}
      >
        {cancelPending ? 'Cancelling…' : 'Confirm cancellation'}
      </button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .operations-scope {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-bottom: 20px;
    padding: 4px;
    width: fit-content;
    max-width: 100%;
    background: var(--surface-raised);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-md);
  }

  .operations-scope a {
    padding: 7px 10px;
    color: var(--text-secondary);
    border-radius: calc(var(--radius-md) - 3px);
    font-size: var(--text-compact);
    font-weight: 520;
  }

  .operations-scope a.active {
    color: var(--text-primary);
    background: var(--surface-base);
    box-shadow: 0 0 0 1px var(--border-subtle);
  }

  .operations-scope > span {
    padding: 0 8px;
    color: var(--text-tertiary);
    font-size: var(--text-meta);
    white-space: nowrap;
  }

  .customer-link {
    color: var(--text-secondary);
    font-weight: 500;
  }

  .customer-link:hover {
    color: var(--accent);
  }

  .managed-context {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin-top: 16px;
    padding: 14px 16px;
    background: var(--accent-soft, var(--surface-raised));
    border: 1px solid var(--border-default);
    border-radius: var(--radius-lg);
  }

  .managed-context > div {
    display: grid;
    gap: 3px;
  }

  .managed-context span {
    color: var(--text-secondary);
    font-size: var(--text-meta);
  }

  .print-fields {
    display: grid;
    gap: 16px;
  }

  .print-fields :global(.ui-select),
  .file-input {
    width: 100%;
  }

  .print-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 100px;
    gap: 12px;
  }

  .profile-summary {
    margin: -8px 0 0;
    color: var(--text-secondary);
    font-size: var(--text-meta);
  }

  .setup {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 16px;
    margin-top: 16px;
    padding: 14px 16px;
    background: var(--surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
  }

  .setup-intro {
    display: grid;
    gap: 2px;
  }

  .setup-intro strong {
    font-size: var(--text-section);
    font-weight: 560;
  }

  .setup-intro span {
    color: var(--text-secondary);
    font-size: var(--text-meta);
  }

  .setup ol {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 18px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .setup li {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--text-compact);
  }

  .step {
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    flex: 0 0 auto;
    color: var(--text-tertiary);
    background: var(--surface-raised);
    border: 1px solid var(--border-default);
    border-radius: 50%;
    font-size: var(--text-meta);
  }

  li.complete .step {
    color: var(--success);
    background: var(--success-soft);
    border-color: transparent;
  }

  li.current .step {
    color: white;
    background: var(--accent);
    border-color: var(--accent);
  }

  .setup a {
    color: var(--text-secondary);
  }

  .setup li.current a {
    color: var(--text-primary);
    font-weight: 500;
  }

  .setup a:hover {
    color: var(--text-primary);
  }

  .metrics {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 8px;
    margin-top: 8px;
    border-bottom: 1px solid var(--border-subtle);
  }

  /* Drawer content */
  .drawer-status {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: var(--text-compact);
  }

  .drawer-section h3 {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 0 0 12px;
    color: var(--text-secondary);
    font-size: var(--text-compact);
    font-weight: 550;
  }

  .drawer-section h3 span {
    color: var(--text-tertiary);
    font-weight: 450;
  }

  .profile,
  .mini-row {
    min-height: var(--row-normal);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    padding: 8px 0;
    border-bottom: 1px solid var(--border-subtle);
    font-size: var(--text-compact);
  }

  .profile > div {
    display: grid;
    line-height: var(--text-compact-line);
  }

  .profile strong {
    font-weight: 500;
  }

  .profile small {
    color: var(--text-tertiary);
    font-size: var(--text-meta);
    text-transform: capitalize;
  }

  .mini-row span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mini-row:hover {
    color: var(--accent);
  }

  .empty-line {
    margin: 0;
    font-size: var(--text-compact);
  }

  form {
    display: grid;
    gap: 12px;
  }

  .connect-steps {
    display: grid;
    gap: 12px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .connect-steps li {
    display: grid;
    grid-template-columns: 24px minmax(0, 1fr);
    align-items: start;
    gap: 10px;
  }

  .connect-steps li > span {
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-default);
    border-radius: 50%;
    font-size: var(--text-meta);
  }

  .connect-steps div {
    display: grid;
    gap: 2px;
  }

  .connect-steps strong {
    font-size: var(--text-compact);
    font-weight: 550;
  }

  .connect-steps small,
  .advanced-options small {
    color: var(--text-tertiary);
    font-size: var(--text-meta);
    line-height: var(--text-compact-line);
  }

  .connect-steps a {
    color: var(--accent);
  }

  .advanced-options {
    padding-top: 2px;
  }

  .advanced-options summary {
    color: var(--text-secondary);
    cursor: pointer;
    font-size: var(--text-compact);
  }

  .advanced-options :global(.field) {
    margin-top: 12px;
  }

  .connection-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
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

  .secret .button {
    justify-self: start;
  }

  @media (max-width: 900px) {
    .metrics {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }

  @media (max-width: 620px) {
    .operations-scope {
      align-items: stretch;
      width: 100%;
      flex-wrap: wrap;
    }

    .operations-scope a {
      flex: 1;
      text-align: center;
    }

    .operations-scope > span {
      width: 100%;
      padding-block: 4px;
      text-align: center;
    }

    .setup {
      align-items: flex-start;
      flex-direction: column;
    }

    .setup ol {
      gap: 12px;
      flex-direction: column;
      align-items: flex-start;
    }
  }
</style>

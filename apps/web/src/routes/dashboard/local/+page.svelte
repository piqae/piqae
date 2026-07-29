<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Icon from '$lib/components/Icon.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';

  type Connection = 'local_only' | 'connected' | 'connecting' | 'offline' | 'degraded';
  type ProfileOptions = {
    copies?: number;
    color?: boolean;
    duplex?: 'one-sided' | 'long-edge' | 'short-edge';
    paper?: string;
    media?: string;
    bin?: string;
    collate?: boolean;
    nup?: number;
    dpi?: string;
    fit_to_page?: boolean;
    native_options?: Record<string, string>;
  };
  type ProfileStatus =
    | 'draft'
    | 'capturing'
    | 'ready'
    | 'needs_test'
    | 'stale'
    | 'driver_mismatch'
    | 'destination_missing'
    | 'dependency_missing'
    | 'interactive_only'
    | 'invalid'
    | 'retired';
  type ProfileSummary = {
    paper?: string | null;
    dimensions_mm?: [number, number] | null;
    source?: string | null;
    media?: string | null;
    color?: string | null;
    duplex?: string | null;
    resolution?: string | null;
    copies?: number | null;
    native?: Record<string, string>;
    details?: Record<string, unknown>;
  };
  type Profile = {
    profile_id: string;
    name: string;
    is_default: boolean;
    revision: number;
    options: ProfileOptions;
    status: ProfileStatus;
    native_kind?: string | null;
    driver_fingerprint?: {
      platform?: string;
      driver_name?: string;
      driver_version?: string | null;
      native_queue_id?: string;
    };
    summary?: ProfileSummary;
    stock_id?: string | null;
    safe_overrides?: string[];
    last_validated_unix_ms?: number | null;
    last_test_job_id?: string | null;
    published?: boolean;
  };
  type Printer = {
    printer_id: string;
    native_id: string;
    name: string;
    state: string;
    is_default: boolean;
    exposed: boolean;
    capability_revision: number;
    capabilities: {
      color?: boolean;
      copies?: number;
      duplex?: boolean;
      papers?: Record<string, unknown>;
      dpis?: string[];
      bins?: string[];
      medias?: string[];
      nup?: number[];
      collate?: boolean;
    };
    native_options: Record<string, unknown>;
    profiles: Profile[];
    queue_counts: { queued?: number; active?: number };
  };
  type LocalStatus = {
    agent_id: string | null;
    workspace_name: string | null;
    version: string;
    connection: Connection;
    queued_jobs: number;
    active_jobs: number;
    printer_warnings: number;
    paused: boolean;
  };
  type QueueSnapshot = {
    local_jobs: Array<Record<string, unknown>>;
    native_jobs: Array<Record<string, unknown>>;
  };

  let { data } = $props();
  const demo = $derived(data.dashboardMode === 'demo');
  const initialDemo = untrack(() => data.dashboardMode === 'demo');

  const demoStatus: LocalStatus = {
    agent_id: 'agt_demo_mac',
    workspace_name: 'C4 Coffee',
    version: '0.1.0',
    connection: 'connected',
    queued_jobs: 1,
    active_jobs: 1,
    printer_warnings: 0,
    paused: false
  };
  const demoProfiles: Profile[] = [
    {
      profile_id: 'profile_demo_a4',
      name: 'A4 packing slips',
      is_default: true,
      revision: 3,
      options: { copies: 1, color: false, duplex: 'one-sided', paper: 'A4', fit_to_page: true },
      status: 'ready',
      native_kind: 'macos_printcore',
      driver_fingerprint: {
        platform: 'macos',
        driver_name: 'HP Color LaserJet MFP M283fdw',
        driver_version: '3.0',
        native_queue_id: 'Office_Printer'
      },
      summary: {
        paper: 'A4',
        source: 'Auto select',
        media: 'Plain paper',
        color: 'Monochrome',
        duplex: 'One-sided',
        resolution: '600 dpi'
      },
      stock_id: 'stock_a4_plain',
      safe_overrides: ['copies', 'color'],
      last_validated_unix_ms: Date.now() - 240_000,
      last_test_job_id: 'job_demo_printed',
      published: true
    }
  ];
  const demoPrinters: Printer[] = [
    {
      printer_id: 'prt_demo_office',
      native_id: 'Office_Printer',
      name: 'Office Laser',
      state: 'online',
      is_default: true,
      exposed: true,
      capability_revision: 7,
      capabilities: {
        color: true,
        copies: 99,
        duplex: true,
        papers: { A4: [210000, 297000], Letter: [215900, 279400] },
        dpis: ['300', '600']
      },
      native_options: {},
      profiles: demoProfiles,
      queue_counts: { queued: 1, active: 1 }
    },
    {
      printer_id: 'prt_demo_label',
      native_id: 'Zebra_GK420d',
      name: 'Dispatch labels',
      state: 'online',
      is_default: false,
      exposed: false,
      capability_revision: 2,
      capabilities: { color: false, copies: 99, duplex: false, papers: { '4x6': null }, dpis: ['203'] },
      native_options: {},
      profiles: [],
      queue_counts: { queued: 0, active: 0 }
    }
  ];

  let status = $state<LocalStatus | null>(initialDemo ? demoStatus : null);
  let printers = $state<Printer[]>(initialDemo ? demoPrinters : []);
  let selectedPrinterId = $state(demoPrinters[0]?.printer_id ?? '');
  let selectedProfileId = $state(demoProfiles[0]?.profile_id ?? '');
  let queue = $state<QueueSnapshot | null>(
    initialDemo
      ? {
          local_jobs: [{ job_id: 'job_demo_queued', state: 'queued_local', title: 'Packing slip #1842' }],
          native_jobs: [{ native_job_id: 'cups_281', state: 'processing', title: 'Packing slip #1841' }]
        }
      : null
  );
  let loading = $state(!initialDemo);
  let refreshing = $state(false);
  let pending = $state('');
  let errorMessage = $state<string | null>(null);
  let actionError = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let confirmationOpen = $state(false);
  let confirmed = $state(false);

  const selectedPrinter = $derived(
    printers.find((printer) => printer.printer_id === selectedPrinterId) ?? printers[0] ?? null
  );
  const selectedProfiles = $derived(selectedPrinter?.profiles ?? []);
  const selectedProfile = $derived(
    selectedProfiles.find((profile) => profile.profile_id === selectedProfileId) ?? null
  );
  const selectedProfileA4Compatible = $derived(
    !(selectedProfile?.summary?.paper ?? selectedProfile?.options.paper) ||
      isA4Paper(selectedProfile?.summary?.paper ?? selectedProfile?.options.paper ?? '')
  );
  const exposedCount = $derived(printers.filter((printer) => printer.exposed).length);

  function isA4Paper(value: string): boolean {
    return /^(?:(?:iso[_ -])?a4(?:[._-](?:210x297(?:mm)?|fullbleed))?|210x297(?:mm)?)$/i.test(
      value.trim()
    );
  }

  function displayError(value: unknown, fallback: string): string {
    if (value && typeof value === 'object') {
      const record = value as Record<string, unknown>;
      if (typeof record.message === 'string') return record.message;
      if (record.error && typeof record.error === 'object') {
        const nested = record.error as Record<string, unknown>;
        if (typeof nested.message === 'string') return nested.message;
      }
    }
    return fallback;
  }

  async function jsonRequest<T>(path: string, init?: RequestInit): Promise<T> {
    const response = await fetch(path, {
      ...init,
      signal: init?.signal ?? AbortSignal.timeout(10_000),
      headers: init?.body ? { 'content-type': 'application/json', ...init.headers } : init?.headers
    });
    if (!response.ok) {
      let body: unknown;
      try {
        body = await response.json();
      } catch {
        body = null;
      }
      throw new Error(displayError(body, `Request failed (${response.status}).`));
    }
    return response.status === 204 ? (undefined as T) : ((await response.json()) as T);
  }

  function normalizeProfile(profile: Profile & { id?: string }): Profile {
    return {
      ...profile,
      profile_id: profile.profile_id ?? profile.id ?? '',
      status: profile.status ?? 'needs_test',
      options: profile.options ?? {},
      summary: profile.summary ?? {},
      safe_overrides: profile.safe_overrides ?? [],
      published: profile.published ?? false
    };
  }

  function normalizePrinter(printer: Printer): Printer {
    return {
      ...printer,
      native_id: printer.native_id ?? printer.printer_id,
      exposed: printer.exposed ?? false,
      capability_revision: printer.capability_revision ?? 0,
      capabilities: printer.capabilities ?? {},
      native_options: printer.native_options ?? {},
      profiles: (printer.profiles ?? []).map(normalizeProfile),
      queue_counts: printer.queue_counts ?? {}
    };
  }

  async function refresh(showSpinner = false) {
    if (demo || refreshing) return;
    refreshing = true;
    if (showSpinner) loading = true;
    try {
      const [nextStatus, nextPrinters] = await Promise.all([
        jsonRequest<LocalStatus>('/api/local/status'),
        jsonRequest<Printer[]>('/api/local/printers')
      ]);
      status = nextStatus;
      printers = nextPrinters.map(normalizePrinter);
      if (!selectedPrinterId || !printers.some((printer) => printer.printer_id === selectedPrinterId)) {
        selectedPrinterId = printers[0]?.printer_id ?? '';
      }
      const printer = printers.find((candidate) => candidate.printer_id === selectedPrinterId);
      if (!printer?.profiles.some((profile) => profile.profile_id === selectedProfileId)) {
        selectedProfileId =
          printer?.profiles.find((profile) => profile.is_default)?.profile_id ??
          printer?.profiles[0]?.profile_id ??
          '';
      }
      errorMessage = null;
      if (selectedPrinterId) await refreshQueue();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : 'The local agent is unavailable.';
    } finally {
      loading = false;
      refreshing = false;
    }
  }

  async function refreshQueue() {
    if (demo || !selectedPrinterId) return;
    try {
      queue = await jsonRequest<QueueSnapshot>(
        `/api/local/printers/${encodeURIComponent(selectedPrinterId)}/queue`
      );
    } catch {
      queue = null;
    }
  }

  async function setPaused(paused: boolean) {
    if (demo) return;
    notice = null;
    actionError = null;
    pending = paused ? 'pause' : 'resume';
    try {
      await jsonRequest(`/api/local/${paused ? 'pause' : 'resume'}`, { method: 'POST' });
      await refresh();
      notice = paused ? 'Local pickup paused.' : 'Local pickup resumed.';
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'The node could not be updated.';
    } finally {
      pending = '';
    }
  }

  async function setExposure(printer: Printer) {
    if (demo) return;
    notice = null;
    actionError = null;
    pending = `exposure:${printer.printer_id}`;
    try {
      await jsonRequest(`/api/local/printers/${encodeURIComponent(printer.printer_id)}/exposure`, {
        method: 'PUT',
        body: JSON.stringify({ exposed: !printer.exposed })
      });
      await refresh();
      notice = printer.exposed ? `${printer.name} is hidden.` : `${printer.name} is exposed.`;
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'Exposure could not be changed.';
    } finally {
      pending = '';
    }
  }

  function selectPrinter(printer: Printer) {
    selectedPrinterId = printer.printer_id;
    selectedProfileId =
      printer.profiles.find((profile) => profile.is_default)?.profile_id ??
      printer.profiles[0]?.profile_id ??
      '';
    void refreshQueue();
  }

  function profileKind(profile: Profile): string {
    if (!profile.native_kind || profile.native_kind === 'portable_options') return 'Basic fallback';
    if (profile.native_kind === 'macos_printcore') return 'macOS native';
    if (profile.native_kind === 'windows_devmode') return 'Windows native';
    if (profile.native_kind === 'windows_print_ticket') return 'Windows PrintTicket';
    if (profile.native_kind === 'cups_instance') return 'CUPS instance';
    return profile.native_kind.replaceAll('_', ' ');
  }

  function profilePaper(profile: Profile | null): string {
    return profile?.summary?.paper ?? profile?.options.paper ?? 'Driver default';
  }

  function profileFact(profile: Profile, key: keyof ProfileSummary, fallback: string): string {
    const value = profile.summary?.[key];
    return typeof value === 'string' && value ? value : fallback;
  }

  function validatedLabel(profile: Profile): string {
    if (!profile.last_validated_unix_ms) return 'Not validated on this node';
    return `Validated ${new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short'
    }).format(new Date(profile.last_validated_unix_ms))}`;
  }

  async function validateProfile(profile: Profile) {
    if (demo) return;
    actionError = null;
    notice = null;
    pending = `validate:${profile.profile_id}`;
    try {
      const result = await jsonRequest<{ status: ProfileStatus; message?: string | null }>(
        `/api/local/profiles/${encodeURIComponent(profile.profile_id)}/validate`,
        { method: 'POST', body: JSON.stringify({ revision: profile.revision }) }
      );
      notice =
        result.message ??
        (result.status === 'ready'
          ? `${profile.name} matches this queue and driver.`
          : `${profile.name} validation returned ${result.status.replaceAll('_', ' ')}.`);
      await refresh();
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'Profile validation failed.';
    } finally {
      pending = '';
    }
  }

  async function deleteProfile(profile: Profile) {
    if (demo || !selectedPrinter) return;
    if (!window.confirm(`Delete the “${profile.name}” profile? This cannot be undone.`)) return;
    actionError = null;
    pending = `delete:${profile.profile_id}`;
    try {
      await jsonRequest(
        `/api/local/printers/${encodeURIComponent(selectedPrinter.printer_id)}/profiles/${encodeURIComponent(profile.profile_id)}?expected_revision=${profile.revision}`,
        { method: 'DELETE' }
      );
      if (selectedProfileId === profile.profile_id) selectedProfileId = '';
      await refresh();
      notice = 'Profile deleted.';
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'Profile could not be deleted.';
    } finally {
      pending = '';
    }
  }

  async function sendHostedTest() {
    if (demo || !selectedPrinter || !selectedProfile || !confirmed) return;
    actionError = null;
    pending = 'hosted-test';
    try {
      const result = await jsonRequest<{ job_id: string; state: string }>('/api/hosted-test', {
        method: 'POST',
        body: JSON.stringify({
          printer_id: selectedPrinter.printer_id,
          profile_id: selectedProfile.profile_id,
          confirmed
        })
      });
      notice = `A4 test registered as ${result.job_id}. Track it through the Jobs page.`;
      confirmationOpen = false;
      confirmed = false;
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'The hosted test could not be sent.';
    } finally {
      pending = '';
    }
  }

  async function runLocalDiagnostic() {
    if (demo || !selectedPrinter || !selectedProfile) return;
    actionError = null;
    pending = 'diagnostic';
    try {
      const result = await jsonRequest<{ job_id: string; state: string }>(
        `/api/local/printers/${encodeURIComponent(selectedPrinter.printer_id)}/test-page`,
        { method: 'POST', body: JSON.stringify({ profile_id: selectedProfile.profile_id }) }
      );
      notice = `Local diagnostic queued as ${result.job_id}. It did not test hosted delivery.`;
      await refreshQueue();
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'The local diagnostic could not run.';
    } finally {
      pending = '';
    }
  }

  function jobLabel(job: Record<string, unknown>): string {
    return String(job.title ?? job.job_id ?? job.native_job_id ?? job.id ?? 'Queue item');
  }

  function jobState(job: Record<string, unknown>): string {
    return String(job.state ?? job.status ?? 'unknown').replaceAll('_', ' ');
  }

  onMount(() => {
    if (demo) return;
    void refresh(true);
    const timer = setInterval(() => void refresh(), 5_000);
    return () => clearInterval(timer);
  });
</script>

<svelte:head><title>Local node · Piqae</title></svelte:head>

{#snippet actions()}
  <button class="button" onclick={() => refresh(true)} disabled={demo || loading}>
    <Icon name="activity" size={13} /> Refresh
  </button>
  <button
    class="button"
    onclick={() => setPaused(!status?.paused)}
    disabled={demo || !status || pending === 'pause' || pending === 'resume'}
  >
    {status?.paused ? 'Resume pickup' : 'Pause pickup'}
  </button>
{/snippet}

<PageHeader
  eyebrow="This Mac"
  title="Local node"
  description="Native driver profiles, local queue truth, and real-time delivery from Piqae to this Mac."
  {actions}
/>

{#if errorMessage}
  <div class="alert error" role="alert">
    <Icon name="warning" size={14} />
    <div><strong>Local node unavailable</strong><span>{errorMessage}</span></div>
  </div>
{/if}
{#if actionError}
  <div class="alert error" role="alert">
    <Icon name="warning" size={14} />
    <div><strong>Action failed</strong><span>{actionError}</span></div>
    <button class="icon-button dismiss" aria-label="Dismiss action error" onclick={() => (actionError = null)}><Icon name="x" size={12} /></button>
  </div>
{/if}
{#if notice}
  <div class="alert success" role="status" aria-live="polite">
    <Icon name="check" size={14} /><span>{notice}</span>
    <button class="icon-button dismiss" aria-label="Dismiss notification" onclick={() => (notice = null)}><Icon name="x" size={12} /></button>
  </div>
{/if}

<section class="metrics" aria-label="Local node status">
  <article class="panel metric">
    <span>Connection</span>
    <strong class="connection"><i class:online={status?.connection === 'connected'}></i>{status?.connection?.replaceAll('_', ' ') ?? 'Unknown'}</strong>
    <small>{status?.workspace_name ?? 'No workspace linked'}</small>
  </article>
  <article class="panel metric">
    <span>Discovered queues</span>
    <strong>{printers.length}</strong>
    <small>{exposedCount} exposed to Piqae</small>
  </article>
  <article class="panel metric">
    <span>Local durable queue</span>
    <strong>{status?.queued_jobs ?? 0}</strong>
    <small>{status?.active_jobs ?? 0} active</small>
  </article>
  <article class="panel metric">
    <span>Node</span>
    <strong class="mono">v{status?.version ?? '—'}</strong>
    <small>{status?.paused ? 'Pickup paused' : `${status?.printer_warnings ?? 0} warnings`}</small>
  </article>
</section>

<section class="section-heading">
  <div><h2>Installed destinations</h2><p>Queues reported by macOS. Exposure controls what the API can address.</p></div>
  <span>{printers.length} found</span>
</section>

<div class="panel table-panel">
  <table>
    <thead><tr><th>Printer</th><th>Driver state</th><th>Profile</th><th>Queue</th><th>Exposure</th><th></th></tr></thead>
    <tbody>
      {#each printers as printer (printer.printer_id)}
        <tr class:selected={selectedPrinterId === printer.printer_id}>
          <td>
            <button class="printer-name" onclick={() => selectPrinter(printer)}>
              <span class="printer-icon"><Icon name="printers" size={14} /></span>
              <span><strong>{printer.name}</strong><small class="mono">{printer.native_id}</small></span>
            </button>
          </td>
          <td><span class:good={printer.state === 'online'} class="state-dot"></span>{printer.state.replaceAll('_', ' ')}{#if printer.is_default}<small class="tag">Default</small>{/if}</td>
          <td>{printer.profiles.length ? `${printer.profiles.length} named` : 'No profiles'}</td>
          <td class="numeric">{printer.queue_counts.queued ?? 0} queued · {printer.queue_counts.active ?? 0} active</td>
          <td>
            <button
              class:enabled={printer.exposed}
              class="toggle"
              role="switch"
              aria-checked={printer.exposed}
              aria-label={`${printer.exposed ? 'Hide' : 'Expose'} ${printer.name}`}
              disabled={demo || pending === `exposure:${printer.printer_id}`}
              onclick={() => setExposure(printer)}
            ><span></span></button>
          </td>
          <td><button class="button small" onclick={() => { selectPrinter(printer); confirmationOpen = true; confirmed = false; }} disabled={!printer.exposed || !printer.profiles.length}>Send A4 test</button></td>
        </tr>
      {:else}
        <tr><td colspan="6" class="empty">{loading ? 'Discovering installed queues…' : 'No operating-system printers were discovered.'}</td></tr>
      {/each}
    </tbody>
  </table>
</div>

{#if confirmationOpen && selectedPrinter}
  <section class="confirmation panel" aria-label="Confirm A4 test">
    <div>
      <span class="eyebrow">End-to-end delivery</span>
      <h2>Send A4 test to {selectedPrinter.name}</h2>
      <p>The PDF is registered in the hosted durable queue, downloaded by this Mac, handed to the local queue, and reported back.</p>
    </div>
    <label>Print profile<select bind:value={selectedProfileId}>{#each selectedProfiles as profile}<option value={profile.profile_id}>{profile.name} · r{profile.revision}{profile.options.paper && !isA4Paper(profile.options.paper) ? ' · not A4' : ''}</option>{/each}</select></label>
    <div class="test-summary">
      <span><small>Printer</small><strong>{selectedPrinter.name}</strong></span>
      <span><small>Paper / media</small><strong>{profilePaper(selectedProfile)} · {selectedProfile ? profileFact(selectedProfile, 'media', 'Driver default') : 'Driver default'}</strong></span>
      <span><small>Output</small><strong>{selectedProfile ? profileFact(selectedProfile, 'color', 'Driver default') : 'Driver default'} · {selectedProfile ? profileFact(selectedProfile, 'duplex', 'Driver default') : 'Driver default'}</strong></span>
    </div>
    <label class="confirm-check"><input type="checkbox" bind:checked={confirmed} /> I confirm this printer and profile. Physical output will be produced.</label>
    {#if !selectedProfileA4Compatible}<p class="paper-warning">This profile explicitly selects {selectedProfile?.options.paper}. Choose or create an A4 profile.</p>{/if}
    <div class="confirmation-actions">
      <button class="button ghost" onclick={() => (confirmationOpen = false)}>Cancel</button>
      <button class="button primary" onclick={sendHostedTest} disabled={demo || !confirmed || !selectedProfile || !selectedProfileA4Compatible || pending === 'hosted-test'}>{pending === 'hosted-test' ? 'Registering…' : 'Confirm & send A4 test'}</button>
    </div>
    <small class="audit-note">The profile ID and revision are included in job metadata while first-class profile audit fields are finalized.</small>
  </section>
{/if}

<div class="management-grid">
  <section>
    <div class="section-heading">
      <div><h2>Native print profiles</h2><p>Immutable driver settings captured and replayed by this node.</p></div>
      {#if selectedPrinter}<span>{selectedPrinter.name}</span>{/if}
    </div>
    <div class="panel profiles">
      <div class="profile-list">
        {#each selectedProfiles as profile}
          <article class:active={selectedProfileId === profile.profile_id}>
            <button onclick={() => (selectedProfileId = profile.profile_id)}>
              <strong>{profile.name}</strong>
              <span>{profileKind(profile)} · r{profile.revision}</span>
            </button>
            <div>
              <small class:ready={profile.status === 'ready'}>{profile.status.replaceAll('_', ' ')}</small>
              <button class="icon-button danger" aria-label={`Delete ${profile.name}`} onclick={() => deleteProfile(profile)} disabled={demo || pending === `delete:${profile.profile_id}`}><Icon name="x" size={12} /></button>
            </div>
          </article>
        {:else}
          <p class="empty compact">No profiles yet. Add one from the Piqae menu bar app.</p>
        {/each}
      </div>
      <div class="profile-detail">
        {#if selectedProfile}
          <div class="profile-title">
            <span>
              <strong>{selectedProfile.name}</strong>
              <small>{profileKind(selectedProfile)} · revision {selectedProfile.revision}{selectedProfile.is_default ? ' · default' : ''}</small>
            </span>
            <span class:ready={selectedProfile.status === 'ready'} class="status-pill">{selectedProfile.status.replaceAll('_', ' ')}</span>
          </div>

          {#if selectedProfile.native_kind === 'portable_options' || !selectedProfile.native_kind}
            <div class="fallback-note">
              <Icon name="warning" size={13} />
              <span><strong>Basic fallback profile</strong><small>Portable options cannot preserve vendor controls. Recreate this from the menu bar app for exact driver replay.</small></span>
            </div>
          {/if}

          <dl class="profile-facts">
            <div><dt>Paper</dt><dd>{profilePaper(selectedProfile)}</dd></div>
            <div><dt>Stock</dt><dd>{selectedProfile.stock_id ?? 'Not assigned'}</dd></div>
            <div><dt>Source</dt><dd>{profileFact(selectedProfile, 'source', 'Driver default')}</dd></div>
            <div><dt>Media</dt><dd>{profileFact(selectedProfile, 'media', 'Driver default')}</dd></div>
            <div><dt>Color</dt><dd>{profileFact(selectedProfile, 'color', 'Driver default')}</dd></div>
            <div><dt>Duplex</dt><dd>{profileFact(selectedProfile, 'duplex', 'Driver default')}</dd></div>
            <div><dt>Resolution</dt><dd>{profileFact(selectedProfile, 'resolution', 'Driver default')}</dd></div>
            <div><dt>Published</dt><dd>{selectedProfile.published ? 'Available to API' : 'Local only'}</dd></div>
          </dl>

          <div class="driver-line">
            <span><small>Driver</small><strong>{selectedProfile.driver_fingerprint?.driver_name ?? 'Native driver'}</strong></span>
            <span><small>Version</small><strong>{selectedProfile.driver_fingerprint?.driver_version ?? 'Unknown'}</strong></span>
          </div>

          <div class="override-block">
            <small>API-safe overrides</small>
            <div class="chips">
              {#each selectedProfile.safe_overrides ?? [] as override}
                <span>{override.replaceAll('_', ' ')}</span>
              {:else}
                <em>Locked to the captured driver settings</em>
              {/each}
            </div>
          </div>

          <div class="profile-actions">
            <span><small>{validatedLabel(selectedProfile)}</small>{#if selectedProfile.last_test_job_id}<small class="mono">Last test {selectedProfile.last_test_job_id}</small>{/if}</span>
            <button class="button small" onclick={() => validateProfile(selectedProfile)} disabled={demo || pending === `validate:${selectedProfile.profile_id}`}>{pending === `validate:${selectedProfile.profile_id}` ? 'Validating…' : 'Validate profile'}</button>
            <button class="button primary small" onclick={runLocalDiagnostic} disabled={demo || pending === 'diagnostic'}>{pending === 'diagnostic' ? 'Queuing…' : 'Print local test'}</button>
          </div>
        {:else}
          <div class="native-setup">
            <span class="printer-icon"><Icon name="printers" size={16} /></span>
            <strong>Create profiles in the native app</strong>
            <p>Open the Piqae icon in the macOS menu bar, choose this printer, then Add profile. The real driver panel captures paper, trays, finishing, marks, calibration, and vendor settings as one immutable revision.</p>
          </div>
        {/if}
      </div>
    </div>
    <div class="native-guidance">
      <div><strong>Add, edit, or clone a profile</strong><span>Use the Piqae menu bar app. Editing creates a new immutable revision through the printer’s native driver panel.</span></div>
      <span class="shortcut">Piqae menu bar → Printers → {selectedPrinter?.name ?? 'Printer'} → Add profile</span>
    </div>
  </section>

  <section>
    <div class="section-heading">
      <div><h2>Queue truth</h2><p>Piqae’s durable queue beside the native OS spooler.</p></div>
      <button class="button ghost small" onclick={refreshQueue} disabled={demo || !selectedPrinter}>Refresh</button>
    </div>
    <div class="panel queues">
      <div class="queue-column">
        <h3>Local durable queue <span>{queue?.local_jobs.length ?? 0}</span></h3>
        {#each queue?.local_jobs ?? [] as job}
          <div class="queue-job"><span><strong>{jobLabel(job)}</strong><small class="mono">{String(job.job_id ?? '')}</small></span><em>{jobState(job)}</em></div>
        {:else}<p class="empty compact">No locally durable jobs.</p>{/each}
      </div>
      <div class="queue-column">
        <h3>macOS / CUPS <span>{queue?.native_jobs.length ?? 0}</span></h3>
        {#each queue?.native_jobs ?? [] as job}
          <div class="queue-job"><span><strong>{jobLabel(job)}</strong><small class="mono">{String(job.native_job_id ?? job.id ?? '')}</small></span><em>{jobState(job)}</em></div>
        {:else}<p class="empty compact">No jobs reported by the OS queue.</p>{/each}
      </div>
      <div class="diagnostic">
        <div><strong>Two independent queue views</strong><small>The durable Piqae job remains visible beside the macOS/CUPS handoff for accurate failure tracing.</small></div>
        <button class="button small" onclick={refreshQueue} disabled={demo || !selectedPrinter}>Refresh queue</button>
      </div>
    </div>
  </section>
</div>

<style>
  .alert { width: 100%; min-height: 42px; display: flex; align-items: center; gap: 9px; margin-top: 12px; padding: 8px 11px; text-align: left; border: 1px solid; border-radius: var(--radius-md); }
  .alert div { display: grid; gap: 2px; } .alert strong { font-size: 11px; } .alert span { font-size: 10px; }
  .alert.error { color: var(--danger); background: var(--danger-soft); border-color: color-mix(in oklch, var(--danger), transparent 70%); }
  .alert.success { color: var(--success); background: var(--success-soft); border-color: color-mix(in oklch, var(--success), transparent 75%); }
  .alert .dismiss { margin-left: auto; color: currentColor; }
  .metrics { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; margin-top: 14px; }
  .metric { min-height: 92px; display: flex; flex-direction: column; justify-content: center; padding: 13px; }
  .metric > span { color: var(--text-tertiary); font-size: 9px; font-weight: 550; text-transform: uppercase; letter-spacing: .04em; }
  .metric strong { margin-top: 6px; font-size: 17px; font-weight: 560; text-transform: capitalize; }
  .metric small { margin-top: 4px; color: var(--text-secondary); font-size: 10px; }
  .connection { display: flex; align-items: center; gap: 7px; font-size: 13px !important; }
  .connection i { width: 7px; height: 7px; background: var(--warning); border-radius: 50%; box-shadow: 0 0 0 3px var(--warning-soft); }
  .connection i.online { background: var(--success); box-shadow: 0 0 0 3px var(--success-soft); }
  .section-heading { min-height: 58px; display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .section-heading h2 { margin: 0; font-size: 12px; font-weight: 560; }
  .section-heading p { margin: 3px 0 0; color: var(--text-tertiary); font-size: 10px; }
  .section-heading > span { color: var(--text-tertiary); font-size: 10px; }
  .table-panel { overflow-x: auto; } table { width: 100%; min-width: 900px; border-collapse: collapse; font-size: 10px; }
  th { height: 30px; padding: 0 11px; color: var(--text-tertiary); font-size: 9px; font-weight: 500; text-align: left; text-transform: uppercase; letter-spacing: .04em; border-bottom: 1px solid var(--border-subtle); }
  td { height: 58px; padding: 0 11px; color: var(--text-secondary); border-bottom: 1px solid var(--border-subtle); }
  tr:last-child td { border-bottom: 0; } tr.selected { background: color-mix(in oklch, var(--surface-selected), transparent 65%); }
  .printer-name { display: flex; align-items: center; gap: 9px; padding: 0; color: inherit; text-align: left; background: none; border: 0; cursor: pointer; }
  .printer-name > span:last-child { display: grid; gap: 2px; } .printer-name strong { color: var(--text-primary); font-size: 11px; font-weight: 520; }
  .printer-name small { max-width: 230px; overflow: hidden; color: var(--text-tertiary); text-overflow: ellipsis; }
  .printer-icon { width: 28px; height: 28px; display: grid; flex: 0 0 auto; place-items: center; background: var(--surface-raised); border: 1px solid var(--border-subtle); border-radius: 7px; }
  .state-dot { width: 6px; height: 6px; display: inline-block; margin-right: 6px; background: var(--warning); border-radius: 50%; } .state-dot.good { background: var(--success); }
  .tag { margin-left: 7px; padding: 2px 4px; color: var(--text-tertiary); border: 1px solid var(--border-default); border-radius: 4px; }
  .toggle { width: 30px; height: 17px; position: relative; padding: 2px; background: var(--surface-raised); border: 1px solid var(--border-strong); border-radius: 99px; cursor: pointer; }
  .toggle span { width: 11px; height: 11px; display: block; background: var(--text-tertiary); border-radius: 50%; transition: transform 120ms ease; }
  .toggle.enabled { background: var(--accent); border-color: var(--accent); } .toggle.enabled span { background: white; transform: translateX(13px); }
  .empty { height: 68px; color: var(--text-tertiary); text-align: center; } .empty.compact { height: auto; padding: 18px 8px; font-size: 10px; }
  .confirmation { display: grid; grid-template-columns: minmax(240px, 1fr) 180px minmax(280px, 1fr); align-items: center; gap: 16px; margin-top: 10px; padding: 14px; border-color: color-mix(in oklch, var(--accent), transparent 65%); }
  .eyebrow { color: var(--accent); font-size: 9px; font-weight: 600; text-transform: uppercase; letter-spacing: .05em; }
  .confirmation h2 { margin: 3px 0; font-size: 13px; } .confirmation p { margin: 0; color: var(--text-secondary); font-size: 10px; line-height: 15px; }
  label { display: grid; gap: 5px; color: var(--text-secondary); font-size: 10px; }
  input, select { width: 100%; height: 29px; padding: 0 8px; color: var(--text-primary); background: var(--surface-raised); border: 1px solid var(--border-default); border-radius: var(--radius-md); font-size: 10px; }
  .test-summary { display: grid; grid-template-columns: repeat(3, 1fr); gap: 5px; }
  .test-summary span { min-width: 0; display: grid; gap: 2px; padding: 6px; background: var(--surface-raised); border: 1px solid var(--border-subtle); border-radius: 5px; }
  .test-summary small { color: var(--text-tertiary); font-size: 8px; text-transform: uppercase; letter-spacing: .03em; }
  .test-summary strong { overflow: hidden; font-size: 9px; font-weight: 520; text-overflow: ellipsis; white-space: nowrap; }
  .confirm-check { display: flex; align-items: center; gap: 7px; line-height: 15px; } .confirm-check input { width: 13px; height: 13px; flex: 0 0 auto; }
  .paper-warning { margin: 0; padding: 7px; color: var(--warning); background: var(--warning-soft); border-radius: 5px; font-size: 9px; }
  .confirmation-actions { display: flex; justify-content: flex-end; gap: 6px; } .audit-note { grid-column: 1 / -1; color: var(--text-tertiary); font-size: 9px; }
  .management-grid { display: grid; grid-template-columns: minmax(0, 1.05fr) minmax(0, .95fr); gap: 12px; }
  .profiles { min-height: 414px; display: grid; grid-template-columns: minmax(190px, .8fr) minmax(250px, 1.2fr); overflow: hidden; }
  .profile-list { border-right: 1px solid var(--border-subtle); }
  .profile-list article { min-height: 58px; display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 8px 10px; border-bottom: 1px solid var(--border-subtle); }
  .profile-list article.active { background: var(--surface-selected); } .profile-list article > button:first-child { min-width: 0; display: grid; flex: 1; gap: 3px; padding: 0; text-align: left; background: none; border: 0; cursor: pointer; }
  .profile-list strong { overflow: hidden; font-size: 10px; font-weight: 540; text-overflow: ellipsis; white-space: nowrap; } .profile-list span, .profile-list small { color: var(--text-tertiary); font-size: 9px; }
  .profile-list article > div { display: flex; align-items: center; gap: 5px; }
  .profile-list small.ready { color: var(--success); }
  .icon-button { width: 24px; height: 24px; display: grid; place-items: center; color: var(--text-tertiary); background: transparent; border: 0; border-radius: 5px; cursor: pointer; } .icon-button:hover { background: var(--surface-hover); } .icon-button.danger:hover { color: var(--danger); }
  .profile-detail { min-width: 0; display: grid; align-content: start; gap: 12px; padding: 13px; }
  .profile-title { min-width: 0; display: flex; align-items: flex-start; justify-content: space-between; gap: 10px; padding-bottom: 11px; border-bottom: 1px solid var(--border-subtle); }
  .profile-title > span:first-child { min-width: 0; display: grid; gap: 3px; }
  .profile-title strong { overflow: hidden; font-size: 12px; font-weight: 560; text-overflow: ellipsis; white-space: nowrap; }
  .profile-title small { color: var(--text-tertiary); font-size: 9px; }
  .status-pill { flex: 0 0 auto; padding: 3px 6px; color: var(--warning); background: var(--warning-soft); border-radius: 5px; font-size: 9px; text-transform: capitalize; }
  .status-pill.ready { color: var(--success); background: var(--success-soft); }
  .fallback-note { display: flex; align-items: flex-start; gap: 8px; padding: 8px; color: var(--warning); background: var(--warning-soft); border: 1px solid color-mix(in oklch, var(--warning), transparent 78%); border-radius: var(--radius-md); }
  .fallback-note > span { display: grid; gap: 3px; }
  .fallback-note strong { font-size: 10px; }
  .fallback-note small { color: var(--text-secondary); font-size: 9px; line-height: 14px; }
  .profile-facts { display: grid; grid-template-columns: 1fr 1fr; gap: 0; margin: 0; border: 1px solid var(--border-subtle); border-radius: var(--radius-md); overflow: hidden; }
  .profile-facts div { min-width: 0; display: grid; grid-template-columns: 76px minmax(0, 1fr); gap: 7px; padding: 7px 8px; border-right: 1px solid var(--border-subtle); border-bottom: 1px solid var(--border-subtle); }
  .profile-facts div:nth-child(2n) { border-right: 0; }
  .profile-facts div:nth-last-child(-n + 2) { border-bottom: 0; }
  .profile-facts dt { color: var(--text-tertiary); font-size: 9px; }
  .profile-facts dd { overflow: hidden; margin: 0; color: var(--text-primary); font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
  .driver-line { display: grid; grid-template-columns: 1fr 90px; gap: 8px; }
  .driver-line span { min-width: 0; display: grid; gap: 3px; }
  .driver-line small, .override-block > small { color: var(--text-tertiary); font-size: 8px; font-weight: 550; text-transform: uppercase; letter-spacing: .04em; }
  .driver-line strong { overflow: hidden; font-size: 9px; font-weight: 500; text-overflow: ellipsis; white-space: nowrap; }
  .override-block { display: grid; gap: 6px; }
  .chips { display: flex; flex-wrap: wrap; gap: 5px; }
  .chips span { padding: 3px 6px; color: var(--text-secondary); background: var(--surface-raised); border: 1px solid var(--border-subtle); border-radius: 5px; font-size: 8px; text-transform: capitalize; }
  .chips em { color: var(--text-tertiary); font-size: 9px; font-style: normal; }
  .profile-actions { display: flex; align-items: center; gap: 6px; margin-top: auto; padding-top: 2px; }
  .profile-actions > span { min-width: 0; display: grid; flex: 1; gap: 2px; }
  .profile-actions small { overflow: hidden; color: var(--text-tertiary); font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
  .native-setup { min-height: 350px; display: grid; align-content: center; justify-items: center; gap: 8px; padding: 28px; text-align: center; }
  .native-setup strong { font-size: 11px; }
  .native-setup p { max-width: 390px; margin: 0; color: var(--text-secondary); font-size: 9px; line-height: 15px; }
  .native-guidance { min-height: 62px; display: flex; align-items: center; justify-content: space-between; gap: 14px; margin-top: 8px; padding: 9px 11px; background: color-mix(in oklch, var(--surface-raised), transparent 20%); border: 1px solid var(--border-subtle); border-radius: var(--radius-md); }
  .native-guidance > div { display: grid; gap: 3px; }
  .native-guidance strong { font-size: 10px; font-weight: 540; }
  .native-guidance span { color: var(--text-tertiary); font-size: 9px; line-height: 13px; }
  .native-guidance .shortcut { max-width: 245px; padding: 5px 7px; color: var(--text-secondary); background: var(--surface-base); border: 1px solid var(--border-subtle); border-radius: 5px; font-family: var(--font-mono); }
  .queues { min-height: 414px; display: grid; grid-template-rows: auto auto 1fr; overflow: hidden; }
  .queue-column { min-height: 118px; border-bottom: 1px solid var(--border-subtle); }
  .queue-column h3 { height: 33px; display: flex; align-items: center; justify-content: space-between; margin: 0; padding: 0 11px; color: var(--text-secondary); font-size: 9px; font-weight: 560; text-transform: uppercase; letter-spacing: .04em; border-bottom: 1px solid var(--border-subtle); }
  .queue-column h3 span { color: var(--text-tertiary); }
  .queue-job { min-height: 49px; display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 7px 11px; border-bottom: 1px solid var(--border-subtle); }
  .queue-job > span { min-width: 0; display: grid; gap: 3px; } .queue-job strong { overflow: hidden; font-size: 10px; font-weight: 500; text-overflow: ellipsis; white-space: nowrap; } .queue-job small { color: var(--text-tertiary); font-size: 8px; }
  .queue-job em { padding: 2px 5px; color: var(--info); background: var(--info-soft); border-radius: 4px; font-size: 9px; font-style: normal; white-space: nowrap; }
  .diagnostic { align-self: end; display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 12px; background: color-mix(in oklch, var(--surface-raised), transparent 30%); }
  .diagnostic > div { display: grid; gap: 3px; } .diagnostic strong { font-size: 10px; } .diagnostic small { color: var(--text-tertiary); font-size: 9px; line-height: 14px; }
  @media (max-width: 1050px) { .metrics { grid-template-columns: 1fr 1fr; } .management-grid { grid-template-columns: 1fr; } .confirmation { grid-template-columns: 1fr 1fr; } }
  @media (max-width: 650px) { .metrics { grid-template-columns: 1fr; } .confirmation { grid-template-columns: 1fr; } .profiles { grid-template-columns: 1fr; } .profile-list { border-right: 0; border-bottom: 1px solid var(--border-subtle); } .profile-facts { grid-template-columns: 1fr; } .profile-facts div { border-right: 0; border-bottom: 1px solid var(--border-subtle) !important; } .profile-facts div:last-child { border-bottom: 0 !important; } .native-guidance { align-items: flex-start; flex-direction: column; } .native-guidance .shortcut { max-width: 100%; } }
</style>

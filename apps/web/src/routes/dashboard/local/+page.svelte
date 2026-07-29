<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Icon from '$lib/components/Icon.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';

  type Connection = 'local_only' | 'connected' | 'connecting' | 'offline' | 'degraded';
  type NativeChoice = { value: string; display_name: string };
  type NativeOption = {
    display_name: string;
    default_choice: string | null;
    choices: NativeChoice[];
  };
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
  type Profile = {
    profile_id: string;
    name: string;
    is_default: boolean;
    revision: number;
    options: ProfileOptions;
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
    native_options: Record<string, NativeOption>;
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
      options: { copies: 1, color: false, duplex: 'one-sided', paper: 'A4', fit_to_page: true }
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
  let editingProfileId = $state<string | null>(null);
  let profileName = $state('');
  let profileDefault = $state(false);
  let profileCopies = $state(1);
  let profileColor = $state(false);
  let profileDuplex = $state<'one-sided' | 'long-edge' | 'short-edge'>('one-sided');
  let profilePaper = $state('A4');
  let profileDpi = $state('');
  let profileBin = $state('');
  let profileMedia = $state('');
  let profileNup = $state(1);
  let profileCollate = $state(false);
  let nativeSelections = $state<Record<string, string>>({});

  const selectedPrinter = $derived(
    printers.find((printer) => printer.printer_id === selectedPrinterId) ?? printers[0] ?? null
  );
  const selectedProfiles = $derived(selectedPrinter?.profiles ?? []);
  const selectedProfile = $derived(
    selectedProfiles.find((profile) => profile.profile_id === selectedProfileId) ?? null
  );
  const selectedProfileA4Compatible = $derived(
    !selectedProfile?.options.paper || isA4Paper(selectedProfile.options.paper)
  );
  const exposedCount = $derived(printers.filter((printer) => printer.exposed).length);
  const portableNativeKeys = new Set([
    'pagesize',
    'pageregion',
    'media',
    'duplex',
    'sides',
    'colormodel',
    'colormode',
    'printcolormode',
    'inputslot',
    'outputbin',
    'mediasource',
    'mediatype',
    'resolution',
    'printerresolution',
    'collate',
    'numberup'
  ]);

  function isAdvancedNativeOption(key: string): boolean {
    return !portableNativeKeys.has(key.toLowerCase().replaceAll(/[^a-z0-9]/g, ''));
  }

  function isA4Paper(value: string): boolean {
    return /^(?:(?:iso[_ -])?a4(?:[._-](?:210x297(?:mm)?|fullbleed))?|210x297(?:mm)?)$/i.test(
      value.trim()
    );
  }

  function defaultPaper(): string {
    const papers = Object.keys(selectedPrinter?.capabilities.papers ?? {});
    return papers.find(isA4Paper) ?? papers[0] ?? 'A4';
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
    return { ...profile, profile_id: profile.profile_id ?? profile.id ?? '' };
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

  function resetProfileForm() {
    editingProfileId = null;
    profileName = '';
    profileDefault = false;
    profileCopies = 1;
    profileColor = false;
    profileDuplex = 'one-sided';
    profilePaper = defaultPaper();
    profileDpi = selectedPrinter?.capabilities.dpis?.[0] ?? '';
    profileBin = selectedPrinter?.capabilities.bins?.[0] ?? '';
    profileMedia = selectedPrinter?.capabilities.medias?.[0] ?? '';
    profileNup = selectedPrinter?.capabilities.nup?.[0] ?? 1;
    profileCollate = false;
    nativeSelections = Object.fromEntries(
      Object.entries(selectedPrinter?.native_options ?? {})
        .filter(([key]) => isAdvancedNativeOption(key))
        .map(([key, option]) => [key, option.default_choice ?? option.choices[0]?.value ?? ''])
    );
  }

  function editProfile(profile: Profile) {
    editingProfileId = profile.profile_id;
    profileName = profile.name;
    profileDefault = profile.is_default;
    profileCopies = profile.options.copies ?? 1;
    profileColor = profile.options.color ?? false;
    profileDuplex = profile.options.duplex ?? 'one-sided';
    profilePaper = profile.options.paper ?? defaultPaper();
    profileDpi = profile.options.dpi ?? selectedPrinter?.capabilities.dpis?.[0] ?? '';
    profileBin = profile.options.bin ?? selectedPrinter?.capabilities.bins?.[0] ?? '';
    profileMedia = profile.options.media ?? selectedPrinter?.capabilities.medias?.[0] ?? '';
    profileNup = profile.options.nup ?? selectedPrinter?.capabilities.nup?.[0] ?? 1;
    profileCollate = profile.options.collate ?? false;
    const advancedDefaults = Object.fromEntries(
      Object.entries(selectedPrinter?.native_options ?? {})
        .filter(([key]) => isAdvancedNativeOption(key))
        .map(([key, option]) => [key, option.default_choice ?? option.choices[0]?.value ?? ''])
    );
    nativeSelections = {
      ...advancedDefaults,
      ...Object.fromEntries(
        Object.entries(profile.options.native_options ?? {}).filter(([key]) =>
          isAdvancedNativeOption(key)
        )
      )
    };
  }

  function profileOptions(): ProfileOptions {
    const copies = Number(profileCopies);
    return {
      copies: Math.max(
        1,
        Math.min(Number.isFinite(copies) ? copies : 1, selectedPrinter?.capabilities.copies ?? 99)
      ),
      color: selectedPrinter?.capabilities.color ? profileColor : false,
      duplex: selectedPrinter?.capabilities.duplex ? profileDuplex : 'one-sided',
      paper: profilePaper || undefined,
      dpi: profileDpi || undefined,
      bin: profileBin || undefined,
      media: profileMedia || undefined,
      nup: profileNup || undefined,
      collate: selectedPrinter?.capabilities.collate ? profileCollate : undefined,
      fit_to_page: true,
      native_options: Object.fromEntries(
        Object.entries(nativeSelections).filter(([key, value]) => isAdvancedNativeOption(key) && value)
      )
    };
  }

  async function saveProfile() {
    if (demo || !selectedPrinter || !profileName.trim()) return;
    actionError = null;
    pending = 'profile';
    const current = selectedProfiles.find((profile) => profile.profile_id === editingProfileId);
    const endpoint = current
      ? `/api/local/printers/${encodeURIComponent(selectedPrinter.printer_id)}/profiles/${encodeURIComponent(current.profile_id)}`
      : `/api/local/printers/${encodeURIComponent(selectedPrinter.printer_id)}/profiles`;
    try {
      await jsonRequest(endpoint, {
        method: current ? 'PUT' : 'POST',
        body: JSON.stringify({
          ...(current ? { expected_revision: current.revision } : {}),
          name: profileName.trim(),
          is_default: profileDefault,
          options: profileOptions()
        })
      });
      notice = current ? 'Profile updated.' : 'Profile created.';
      resetProfileForm();
      await refresh();
    } catch (error) {
      actionError = error instanceof Error ? error.message : 'Profile could not be saved.';
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
      resetProfileForm();
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

<svelte:head><title>Local node · Spool</title></svelte:head>

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
  description="Live driver discovery, durable profiles, and both sides of the operating-system queue."
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
    <small>{exposedCount} exposed to Spool</small>
  </article>
  <article class="panel metric">
    <span>Local durable queue</span>
    <strong>{status?.queued_jobs ?? 0}</strong>
    <small>{status?.active_jobs ?? 0} active</small>
  </article>
  <article class="panel metric">
    <span>Agent</span>
    <strong class="mono">v{status?.version ?? '—'}</strong>
    <small>{status?.paused ? 'Pickup paused' : `${status?.printer_warnings ?? 0} warnings`}</small>
  </article>
</section>

<section class="section-heading">
  <div><h2>Discovered OS queues</h2><p>Queues and capabilities reported directly by installed macOS drivers.</p></div>
  <span>{printers.length} found</span>
</section>

<div class="panel table-panel">
  <table>
    <thead><tr><th>Printer</th><th>Driver state</th><th>Profile</th><th>Queue</th><th>Exposure</th><th></th></tr></thead>
    <tbody>
      {#each printers as printer (printer.printer_id)}
        <tr class:selected={selectedPrinterId === printer.printer_id}>
          <td>
            <button class="printer-name" onclick={() => { selectedPrinterId = printer.printer_id; selectedProfileId = printer.profiles.find((profile) => profile.is_default)?.profile_id ?? printer.profiles[0]?.profile_id ?? ''; resetProfileForm(); void refreshQueue(); }}>
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
          <td><button class="button small" onclick={() => { selectedPrinterId = printer.printer_id; selectedProfileId = printer.profiles.find((profile) => profile.is_default)?.profile_id ?? printer.profiles[0]?.profile_id ?? ''; confirmationOpen = true; confirmed = false; }} disabled={!printer.exposed || !printer.profiles.length}>Send A4 test</button></td>
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
      <span><small>Paper / media</small><strong>{selectedProfile?.options.paper ?? 'Driver default'} · {selectedProfile?.options.media ?? 'Default media'}</strong></span>
      <span><small>Output</small><strong>{selectedProfile?.options.color ? 'Color' : 'Mono'} · {selectedProfile?.options.duplex ?? 'one-sided'}</strong></span>
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
      <div><h2>Named profiles</h2><p>Versioned settings persisted by the local agent.</p></div>
      {#if selectedPrinter}<span>{selectedPrinter.name}</span>{/if}
    </div>
    <div class="panel profiles">
      <div class="profile-list">
        {#each selectedProfiles as profile}
          <article class:active={editingProfileId === profile.profile_id}>
            <button onclick={() => editProfile(profile)}>
              <strong>{profile.name}</strong>
              <span>{profile.options.paper ?? 'Driver default'} · {profile.options.color ? 'Color' : 'Mono'} · {profile.options.duplex ?? 'one-sided'}</span>
            </button>
            <div><small>r{profile.revision}{profile.is_default ? ' · Default' : ''}</small><button class="icon-button danger" aria-label={`Delete ${profile.name}`} onclick={() => deleteProfile(profile)} disabled={demo || pending === `delete:${profile.profile_id}`}><Icon name="x" size={12} /></button></div>
          </article>
        {:else}
          <p class="empty compact">Select a printer and create its first profile.</p>
        {/each}
      </div>
      <form onsubmit={(event) => { event.preventDefault(); void saveProfile(); }}>
        <div class="form-title"><strong>{editingProfileId ? 'Edit profile' : 'New profile'}</strong>{#if editingProfileId}<button type="button" class="button ghost small" onclick={resetProfileForm}>New</button>{/if}</div>
        <label>Name<input bind:value={profileName} maxlength="80" placeholder="A4 packing slips" required /></label>
        <div class="form-row">
          <label>Paper<select bind:value={profilePaper}>{#each Object.keys(selectedPrinter?.capabilities.papers ?? {}) as paper}<option value={paper}>{paper}</option>{/each}{#if !Object.keys(selectedPrinter?.capabilities.papers ?? {}).length}<option value="A4">A4</option>{/if}</select></label>
          <label>Copies<input type="number" min="1" max={selectedPrinter?.capabilities.copies ?? 99} bind:value={profileCopies} /></label>
        </div>
        <div class="form-row">
          <label>Color<select bind:value={profileColor} disabled={!selectedPrinter?.capabilities.color}><option value={false}>Monochrome</option><option value={true}>Color</option></select></label>
          <label>Duplex<select bind:value={profileDuplex} disabled={!selectedPrinter?.capabilities.duplex}><option value="one-sided">One-sided</option><option value="long-edge">Long edge</option><option value="short-edge">Short edge</option></select></label>
        </div>
        {#if selectedPrinter?.capabilities.dpis?.length}
          <label>Resolution<select bind:value={profileDpi}>{#each selectedPrinter.capabilities.dpis as dpi}<option value={dpi}>{dpi} dpi</option>{/each}</select></label>
        {/if}
        {#if selectedPrinter?.capabilities.medias?.length || selectedPrinter?.capabilities.bins?.length}
          <div class="form-row">
            {#if selectedPrinter.capabilities.medias?.length}<label>Media type<select bind:value={profileMedia}>{#each selectedPrinter.capabilities.medias as media}<option value={media}>{media}</option>{/each}</select></label>{/if}
            {#if selectedPrinter.capabilities.bins?.length}<label>Paper source / bin<select bind:value={profileBin}>{#each selectedPrinter.capabilities.bins as bin}<option value={bin}>{bin}</option>{/each}</select></label>{/if}
          </div>
        {/if}
        {#if selectedPrinter?.capabilities.nup?.length || selectedPrinter?.capabilities.collate}
          <div class="form-row">
            {#if selectedPrinter.capabilities.nup?.length}<label>Pages per sheet<select bind:value={profileNup}>{#each selectedPrinter.capabilities.nup as nup}<option value={nup}>{nup}</option>{/each}</select></label>{/if}
            {#if selectedPrinter.capabilities.collate}<label class="check"><input type="checkbox" bind:checked={profileCollate} /> Collate copies</label>{/if}
          </div>
        {/if}
        {#each Object.entries(selectedPrinter?.native_options ?? {}).filter(([key]) => isAdvancedNativeOption(key)) as [key, option]}
          <label>{option.display_name}<select value={nativeSelections[key] ?? option.default_choice ?? ''} onchange={(event) => (nativeSelections = { ...nativeSelections, [key]: event.currentTarget.value })}>{#each option.choices as choice}<option value={choice.value}>{choice.display_name}</option>{/each}</select></label>
        {/each}
        <label class="check"><input type="checkbox" bind:checked={profileDefault} /> Default for this printer</label>
        <button class="button primary" type="submit" disabled={demo || !selectedPrinter || !profileName.trim() || pending === 'profile'}>{pending === 'profile' ? 'Saving…' : editingProfileId ? 'Save revision' : 'Create profile'}</button>
      </form>
    </div>
  </section>

  <section>
    <div class="section-heading">
      <div><h2>Queue truth</h2><p>Spool’s durable queue beside the native OS spooler.</p></div>
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
        <div><strong>Local-only diagnostic</strong><small>Bypasses the hosted queue and tests only agent → driver → OS handoff.</small></div>
        <button class="button small" onclick={runLocalDiagnostic} disabled={demo || !selectedProfile || pending === 'diagnostic'}>{pending === 'diagnostic' ? 'Queuing…' : 'Run diagnostic'}</button>
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
  .confirm-check, .check { display: flex; align-items: center; gap: 7px; line-height: 15px; } .confirm-check input, .check input { width: 13px; height: 13px; flex: 0 0 auto; }
  .paper-warning { margin: 0; padding: 7px; color: var(--warning); background: var(--warning-soft); border-radius: 5px; font-size: 9px; }
  .confirmation-actions { display: flex; justify-content: flex-end; gap: 6px; } .audit-note { grid-column: 1 / -1; color: var(--text-tertiary); font-size: 9px; }
  .management-grid { display: grid; grid-template-columns: minmax(0, 1.05fr) minmax(0, .95fr); gap: 12px; }
  .profiles { min-height: 414px; display: grid; grid-template-columns: minmax(190px, .8fr) minmax(250px, 1.2fr); overflow: hidden; }
  .profile-list { border-right: 1px solid var(--border-subtle); }
  .profile-list article { min-height: 58px; display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 8px 10px; border-bottom: 1px solid var(--border-subtle); }
  .profile-list article.active { background: var(--surface-selected); } .profile-list article > button:first-child { min-width: 0; display: grid; flex: 1; gap: 3px; padding: 0; text-align: left; background: none; border: 0; cursor: pointer; }
  .profile-list strong { overflow: hidden; font-size: 10px; font-weight: 540; text-overflow: ellipsis; white-space: nowrap; } .profile-list span, .profile-list small { color: var(--text-tertiary); font-size: 9px; }
  .profile-list article > div { display: flex; align-items: center; gap: 5px; }
  .icon-button { width: 24px; height: 24px; display: grid; place-items: center; color: var(--text-tertiary); background: transparent; border: 0; border-radius: 5px; cursor: pointer; } .icon-button:hover { background: var(--surface-hover); } .icon-button.danger:hover { color: var(--danger); }
  form { display: grid; align-content: start; gap: 10px; padding: 12px; } .form-title { display: flex; align-items: center; justify-content: space-between; min-height: 25px; } .form-title strong { font-size: 11px; }
  .form-row { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
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
  @media (max-width: 650px) { .metrics { grid-template-columns: 1fr; } .confirmation { grid-template-columns: 1fr; } .profiles { grid-template-columns: 1fr; } .profile-list { border-right: 0; border-bottom: 1px solid var(--border-subtle); } }
</style>

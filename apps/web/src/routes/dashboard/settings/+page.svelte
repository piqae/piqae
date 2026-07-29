<script lang="ts">
  import PageHeader from '$lib/components/PageHeader.svelte';
  let retention = $state('24');
  let raw = $state(true);
  let privateUris = $state(true);
</script>

<svelte:head><title>Settings · Spool</title></svelte:head>

<PageHeader
  title="Settings"
  description="Workspace policy, data retention, and deployment configuration."
/>

<div class="settings-grid">
  <nav class="settings-nav" aria-label="Settings sections">
    <a class="active" href="#general">General</a>
    <a href="#printing">Printing policy</a>
    <a href="#retention">Data retention</a>
    <a href="#team">Team</a>
    <a href="#audit">Audit log</a>
    <a href="#usage">Usage</a>
  </nav>

  <div class="settings-content">
    <section class="panel" id="general">
      <header><h2>Workspace</h2><p>The tenancy and environment boundary shown to your team.</p></header>
      <div class="form-body">
        <label class="field"><span>Name</span><input class="input" value="C4 Coffee" disabled /></label>
        <label class="field"><span>Slug</span><input class="input mono" value="c4-coffee" disabled /></label>
        <label class="field"><span>Default region</span><input class="input" value="Sydney (syd1)" disabled /></label>
      </div>
      <footer><button class="button primary" disabled title="Workspace mutation is not implemented">Save changes</button></footer>
    </section>

    <section class="panel" id="printing">
      <header><h2>Printing policy</h2><p>Guardrails applied before a job can enter a local queue.</p></header>
      <div class="toggle-list">
        <label>
          <span><strong>Allow RAW printing</strong><small>Permit unrendered printer-language payloads.</small></span>
          <input type="checkbox" bind:checked={raw} disabled />
        </label>
        <label>
          <span><strong>Allow private URI sources</strong><small>Agents may fetch documents from private network ranges.</small></span>
          <input type="checkbox" bind:checked={privateUris} disabled />
        </label>
        <label>
          <span><strong>Require manual uncertain resolution</strong><small>Never retry when OS handoff cannot be proven.</small></span>
          <input type="checkbox" checked disabled />
        </label>
      </div>
    </section>

    <section class="panel" id="retention">
      <header><h2>Document retention</h2><p>Content is encrypted at rest and deleted independently of job metadata.</p></header>
      <div class="form-body">
        <label class="field">
          <span>Delete successful job content after</span>
          <select class="input" bind:value={retention} disabled>
            <option value="1">1 hour</option>
            <option value="24">24 hours</option>
            <option value="72">3 days</option>
            <option value="168">7 days</option>
          </select>
        </label>
      </div>
      <footer><button class="button primary" disabled title="Retention mutation is not implemented">Save retention</button></footer>
    </section>
  </div>
</div>

<style>
  .settings-grid {
    display: grid;
    grid-template-columns: 180px minmax(0, 720px);
    gap: 28px;
    padding-top: 18px;
  }

  .settings-nav {
    position: sticky;
    top: 18px;
    align-self: start;
    display: grid;
    gap: 2px;
  }

  .settings-nav a {
    height: 29px;
    display: flex;
    align-items: center;
    padding: 0 8px;
    color: var(--text-tertiary);
    border-radius: var(--radius-md);
    font-size: 10px;
  }

  .settings-nav a:hover,
  .settings-nav a.active {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .settings-content {
    display: grid;
    gap: 12px;
  }

  section > header {
    padding: 13px 14px;
    border-bottom: 1px solid var(--border-subtle);
  }

  h2 {
    margin: 0;
    font-size: 11px;
    font-weight: 550;
  }

  header p {
    margin: 2px 0 0;
    color: var(--text-tertiary);
    font-size: 9px;
  }

  .form-body {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    padding: 14px;
  }

  .field:last-child:nth-child(odd) {
    grid-column: 1 / -1;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    padding: 9px 14px;
    background: color-mix(in oklch, var(--canvas), transparent 40%);
    border-top: 1px solid var(--border-subtle);
  }

  .toggle-list label {
    min-height: 55px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    padding: 9px 14px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .toggle-list label:last-child {
    border-bottom: 0;
  }

  .toggle-list label > span {
    display: grid;
  }

  .toggle-list strong {
    font-size: 10px;
    font-weight: 520;
  }

  .toggle-list small {
    margin-top: 2px;
    color: var(--text-tertiary);
    font-size: 9px;
  }

  input[type='checkbox'] {
    width: 30px;
    height: 17px;
    flex: 0 0 auto;
    appearance: none;
    background: var(--surface-raised);
    border: 1px solid var(--border-default);
    border-radius: 99px;
    cursor: pointer;
  }

  input[type='checkbox']::after {
    width: 11px;
    height: 11px;
    display: block;
    margin: 2px;
    content: '';
    background: var(--text-tertiary);
    border-radius: 50%;
    transition: transform 100ms ease;
  }

  input[type='checkbox']:checked {
    background: var(--accent);
    border-color: var(--accent);
  }

  input[type='checkbox']:checked::after {
    background: white;
    transform: translateX(13px);
  }

  @media (max-width: 720px) {
    .settings-grid {
      grid-template-columns: 1fr;
      gap: 14px;
    }

    .settings-nav {
      position: static;
      display: flex;
      overflow-x: auto;
    }

    .settings-nav a {
      white-space: nowrap;
    }
  }

  @media (max-width: 520px) {
    .form-body {
      grid-template-columns: 1fr;
    }
  }
</style>

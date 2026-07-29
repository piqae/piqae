<script lang="ts">
  import FlowDiagram from '$lib/components/marketing/FlowDiagram.svelte';
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';

  const lifecycle = [
    ['Queued', 'The control plane has durably accepted the job and its idempotency key.'],
    ['Dispatched', 'A lease has been offered to an eligible enrolled agent.'],
    ['Accepted', 'The agent durably owns the job or the OS spooler has accepted the handoff.'],
    ['Uncertain', 'The system cannot safely prove the final result after a boundary failure.'],
    ['Failed', 'A terminal, explainable failure occurred and can be acted on explicitly.']
  ];
</script>

<Seo
  title="How Spool works — From your app to any printer"
  description="See how Spool connects your application to remote printers through one API, resilient queues, a lightweight local agent, and installed OS drivers."
  path="/how-it-works"
/>

<MarketingShell>
  <section class="m-page-hero">
    <div class="m-container">
      <span class="m-eyebrow">How it works</span>
      <h1 class="m-title">From your app to the right printer—without the guesswork.</h1>
      <p class="m-lede">
        One API connects your workflow to printers at every site. Spool manages the journey, the
        local agent works with installed drivers, and your application gets clear status back.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="/start?plan=free&source=how-it-works">Start free</a>
        <a class="m-button" href="/docs/quickstart">Open the quickstart</a>
      </div>
    </div>
  </section>

  <section class="m-container">
    <div class="m-dark-panel"><FlowDiagram /></div>
  </section>

  <section class="m-section">
    <div class="m-container process">
      <article>
        <span>01 · API submission</span>
        <h2>Send the job once.</h2>
        <p>
          Submit PDF, RAW content, or a secure URI. A unique key makes retries safe when a response
          gets lost on the network.
        </p>
        <a href="/docs/idempotency">Idempotency guide →</a>
      </article>
      <article>
        <span>02 · Server queue</span>
        <h2>Keep it moving safely.</h2>
        <p>
          Spool stores the job before routing it to an eligible printer and agent, so a brief
          outage does not mean starting the workflow again.
        </p>
        <a href="/docs/jobs">Job API →</a>
      </article>
      <article>
        <span>03 · Enrolled agent</span>
        <h2>Reach the printer’s network.</h2>
        <p>
          A securely enrolled native agent receives the job and saves it locally before taking
          responsibility for the final handoff.
        </p>
        <a href="/docs/nodes">Node enrolment →</a>
      </article>
      <article>
        <span>04 · Native handoff</span>
        <h2>Keep every local capability.</h2>
        <p>
          Spool reads printer options from Windows or CUPS. PDF follows the native print path,
          while RAW bytes reach the driver unchanged.
        </p>
        <a href="/docs/printers">Printer capabilities →</a>
      </article>
      <article>
        <span>05 · Recovery</span>
        <h2>Recover with context.</h2>
        <p>
          After a restart, the agent checks its local queue against the operating system. If it
          cannot prove a retry is safe, it asks for attention instead of risking a duplicate.
        </p>
        <a href="/docs/job-statuses">Job statuses →</a>
      </article>
    </div>
  </section>

  <section id="status" class="state-section m-section">
    <div class="m-container">
      <span class="m-eyebrow">Status semantics</span>
      <h2 class="m-heading">A useful answer at every stage.</h2>
      <div class="state-grid">
        {#each lifecycle as state, index}
          <article>
            <span>{String(index + 1).padStart(2, '0')}</span>
            <h3>{state[0]}</h3>
            <p>{state[1]}</p>
          </article>
        {/each}
      </div>
      <div class="m-note">
        <strong>Physical delivery boundary.</strong>
        <span>
          “Accepted by spooler” means the operating system accepted the handoff. It does not prove
          paper moved, ink landed, or a person received the output.
        </span>
      </div>
    </div>
  </section>

  <section class="m-section">
    <div class="m-container">
      <span class="m-eyebrow">Deployment topologies</span>
      <h2 class="m-heading">The same simple model, wherever you run it.</h2>
      <div class="topologies">
        <article class="m-card">
          <span>Cloud</span>
          <code>Your app → Spool Cloud → agent → OS</code>
          <p>Managed control plane and billing, with the native execution boundary on your network.</p>
        </article>
        <article class="m-card">
          <span>Self-hosted</span>
          <code>Your app → your Spool stack → agent → OS</code>
          <p>Operate the control plane and storage yourself under the Apache-2.0 licence.</p>
        </article>
        <article class="m-card">
          <span>Local-only</span>
          <code>Your app → local agent → OS</code>
          <p>Keep the workflow inside a trusted network when cloud coordination is unnecessary.</p>
        </article>
      </div>
    </div>
  </section>
</MarketingShell>

<style>
  .process { display: grid; gap: 0; }
  .process article {
    display: grid;
    grid-template-columns: 160px 1fr 1fr;
    gap: clamp(25px, 5vw, 75px);
    padding: 45px 0;
    border-top: 1px solid var(--m-border);
  }
  .process article > span { color: var(--m-violet-dark); font: 11px var(--font-mono); }
  .process h2 { margin: 0; font-size: 30px; line-height: 1.1; letter-spacing: -.04em; }
  .process p { margin: 0; color: var(--m-muted); }
  .process a { grid-column: 3; color: var(--m-violet-dark); font-size: 13px; font-weight: 650; }
  .state-section { background: #eeece6; }
  .state-grid {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 8px;
    margin: 40px 0 18px;
  }
  .state-grid article {
    min-height: 225px;
    padding: 21px;
    border: 1px solid var(--m-border);
    border-radius: 14px;
    background: rgb(255 255 255 / .55);
  }
  .state-grid span { color: var(--m-faint); font: 10px var(--font-mono); }
  .state-grid h3 { margin: 70px 0 8px; font-size: 17px; }
  .state-grid p { margin: 0; color: var(--m-muted); font-size: 12px; }
  .topologies { display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; margin-top: 40px; }
  .topologies article { display: grid; gap: 18px; }
  .topologies article > span { color: var(--m-violet-dark); font-size: 11px; font-weight: 700; text-transform: uppercase; }
  .topologies code { overflow-x: auto; color: var(--m-ink); font: 12px/1.6 var(--font-mono); }
  @media (max-width: 900px) {
    .state-grid { grid-template-columns: repeat(2, 1fr); }
    .topologies { grid-template-columns: 1fr; }
  }
  @media (max-width: 680px) {
    .process article { grid-template-columns: 1fr; gap: 14px; }
    .process a { grid-column: 1; }
    .state-grid { grid-template-columns: 1fr; }
    .state-grid article { min-height: 180px; }
  }
</style>

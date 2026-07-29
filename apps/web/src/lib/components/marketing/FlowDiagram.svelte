<script lang="ts">
  let { compact = false }: { compact?: boolean } = $props();
  const steps = [
    { label: 'Application', detail: 'PDF or RAW' },
    { label: 'Spool API', detail: 'Idempotent request' },
    { label: 'Durable queue', detail: 'Lease + recovery' },
    { label: 'Native agent', detail: 'Local handoff' },
    { label: 'OS spooler', detail: 'Accepted ≠ printed' }
  ];
</script>

<div class="flow" class:compact aria-label="Printing flow from application to operating system spooler">
  <div class="flow-top">
    <span class="window-dots"><i></i><i></i><i></i></span>
    <span>live workflow</span>
    <span class="online"><i></i> Agent online</span>
  </div>
  <div class="flow-body">
    {#each steps as step, index}
      <div class="step">
        <span class="index">{String(index + 1).padStart(2, '0')}</span>
        <strong>{step.label}</strong>
        <small>{step.detail}</small>
      </div>
      {#if index < steps.length - 1}<span class="connector" aria-hidden="true">→</span>{/if}
    {/each}
  </div>
  <div class="event-log" aria-hidden="true">
    <span><i class="green"></i> job.created</span>
    <span><i class="violet"></i> agent.accepted</span>
    <span><i class="amber"></i> spooler.accepted</span>
    <span class="muted-event">physical result unknown</span>
  </div>
</div>

<style>
  .flow { padding: 14px; }
  .flow-top {
    min-height: 40px;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 0 5px 13px;
    border-bottom: 1px solid var(--m-border-light);
    color: #777681;
    font: 11px var(--font-mono);
  }
  .window-dots { display: flex; gap: 5px; }
  .window-dots i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #34343d;
  }
  .online { justify-self: end; color: #a7a5ae; }
  .online i {
    width: 6px;
    height: 6px;
    display: inline-block;
    margin-right: 5px;
    border-radius: 50%;
    background: var(--m-green);
    box-shadow: 0 0 8px rgb(62 187 129 / 0.7);
  }
  .flow-body {
    display: grid;
    grid-template-columns: 1fr auto 1fr auto 1fr auto 1fr auto 1fr;
    align-items: center;
    gap: 9px;
    min-height: 260px;
    padding: 34px 18px;
  }
  .step {
    min-width: 0;
    min-height: 118px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 16px;
    border: 1px solid var(--m-border-light);
    border-radius: 12px;
    background: linear-gradient(145deg, #1b1c23, #121319);
    box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.035);
  }
  .index {
    margin-bottom: 16px;
    color: #579cff;
    font: 10px var(--font-mono);
  }
  strong { color: #f5f4f7; font-size: 13px; font-weight: 580; }
  small { margin-top: 4px; color: #777681; font: 10px/1.4 var(--font-mono); }
  .connector { color: #4d4c57; }
  .event-log {
    min-height: 38px;
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 0 12px;
    border-top: 1px solid var(--m-border-light);
    color: #8c8a94;
    font: 10px var(--font-mono);
  }
  .event-log span { white-space: nowrap; }
  .event-log i {
    width: 5px;
    height: 5px;
    display: inline-block;
    margin-right: 5px;
    border-radius: 50%;
  }
  .green { background: var(--m-green); }
  .violet { background: var(--m-violet); }
  .amber { background: #d7a84a; }
  .muted-event { margin-left: auto; color: #64636d; }
  .compact .flow-body { min-height: 210px; }
  @media (max-width: 880px) {
    .flow-body {
      grid-template-columns: 1fr;
      padding: 24px 8px;
    }
    .step { min-height: 76px; }
    .index { margin-bottom: 7px; }
    .connector { justify-self: center; transform: rotate(90deg); }
    .event-log { overflow-x: auto; }
    .muted-event { margin-left: 0; }
  }
</style>

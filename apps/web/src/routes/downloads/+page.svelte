<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  let { data } = $props();
  const releases = 'https://github.com/C4CoffeeCo/spool/releases/latest';
</script>

<svelte:head>
  <title>Download Spool</title>
  <meta name="description" content="Install the small native Spool node for Windows or macOS." />
</svelte:head>

<header class="topbar">
  <a class="brand" href="/dashboard"><span><Icon name="printers" size={14} /></span>Spool</a>
  <a class="button" href="/dashboard/nodes">Back to nodes</a>
</header>

<main>
  <div class="intro">
    <span class="eyebrow">Native nodes</span>
    <h1>Connect a printer computer</h1>
    <p>Install Spool where the printer driver already works. The node keeps its durable queue locally and uses the operating system’s native drivers.</p>
  </div>

  <section class="downloads" aria-label="Node downloads">
    <article class="panel">
      <span class="platform"><Icon name="agents" size={19} /></span>
      <div><h2>Windows</h2><p>64-bit installer · native tray app, service, Winspool, and PDFium renderer.</p></div>
      <a class="button primary" href={releases}>View Windows release <Icon name="external" size={12} /></a>
    </article>
    <article class="panel">
      <span class="platform"><Icon name="agents" size={19} /></span>
      <div><h2>macOS</h2><p>Apple silicon and Intel · native menu bar app with PrintCore profile capture.</p></div>
      <a class="button primary" href={releases}>View macOS release <Icon name="external" size={12} /></a>
    </article>
    <article class="panel">
      <span class="platform"><Icon name="api" size={19} /></span>
      <div><h2>Build from source</h2><p>Apache-2.0 licensed. Audit, modify, and self-host the complete system.</p></div>
      <a class="button" href="https://github.com/C4CoffeeCo/spool">Open repository <Icon name="external" size={12} /></a>
    </article>
  </section>

  <section class="next panel">
    <span>Next</span>
    <div><strong>Add the node to your workspace</strong><p>Create a short-lived token in Spool, paste it into the native app, and wait for printers to appear.</p></div>
    <a class="button" href="/dashboard/nodes">Add node <Icon name="arrow-right" size={12} /></a>
  </section>

  <footer>
    <span>{data.meta.deployment.replace('_', ' ')} · v{data.meta.version}</span>
    <a href="/docs/quickstart">Installation guide</a>
  </footer>
</main>

<style>
  .topbar { height: 54px; display: flex; align-items: center; justify-content: space-between; padding: 0 24px; background: var(--sidebar); border-bottom: 1px solid var(--border-subtle); }
  .brand { display: flex; align-items: center; gap: 8px; font-weight: 580; }
  .brand > span { width: 24px; height: 24px; display: grid; place-items: center; color: white; background: var(--accent); border-radius: 6px; }
  main { width: min(920px, calc(100% - 32px)); margin: 0 auto; padding: 70px 0 36px; }
  .intro { max-width: 650px; }
  .eyebrow { color: var(--accent); font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: .08em; }
  h1 { margin: 10px 0; font-size: clamp(28px, 5vw, 42px); line-height: 1.05; font-weight: 600; letter-spacing: -.045em; }
  .intro p { margin: 0; color: var(--text-secondary); font-size: 13px; line-height: 21px; }
  .downloads { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; margin-top: 35px; }
  article { min-height: 225px; display: flex; flex-direction: column; align-items: flex-start; padding: 16px; }
  .platform { width: 38px; height: 38px; display: grid; place-items: center; color: var(--text-secondary); background: var(--surface-raised); border: 1px solid var(--border-default); border-radius: 9px; }
  article div { flex: 1; padding-top: 20px; }
  h2 { margin: 0; font-size: 13px; font-weight: 560; }
  article p, .next p { margin: 5px 0 0; color: var(--text-tertiary); font-size: 10px; line-height: 16px; }
  article .button { width: 100%; }
  .next { min-height: 76px; display: grid; grid-template-columns: 36px 1fr auto; align-items: center; gap: 14px; margin-top: 12px; padding: 13px 15px; }
  .next > span { width: 34px; height: 22px; display: grid; place-items: center; color: var(--accent); background: var(--accent-soft); border-radius: 6px; font-size: 9px; font-weight: 600; }
  .next strong { font-size: 10px; font-weight: 550; }
  footer { display: flex; justify-content: space-between; padding-top: 24px; color: var(--text-tertiary); font-size: 9px; }
  footer a:hover { color: var(--text-primary); }
  @media (max-width: 760px) { main { padding-top: 42px; } .downloads { grid-template-columns: 1fr; } article { min-height: 185px; } }
  @media (max-width: 520px) { .topbar { padding: 0 14px; } .next { grid-template-columns: 36px 1fr; } .next .button { grid-column: 2; } }
</style>

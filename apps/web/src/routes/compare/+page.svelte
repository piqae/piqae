<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';

  const guides = [
    {
      label: 'Direct comparison',
      title: 'Piqae vs PrintNode',
      body: 'Compare deployment, source access, pricing shape, formats, agent model, and current maturity.',
      href: '/compare/printnode',
      live: true
    },
    {
      label: 'Buyer guide',
      title: 'A PrintNode alternative',
      body: 'Decide whether control, open source, or self-hosting changes the right choice for your workflow.',
      href: '/alternatives/printnode',
      live: true
    },
    {
      label: 'Migration',
      title: 'Move from PrintNode',
      body: 'Inventory endpoints, map semantics, canary printers, and preserve a tested rollback.',
      href: '/migrate/printnode',
      live: true
    },
    {
      label: 'Calculator',
      title: 'Compare estimated costs',
      body: 'Model jobs, agents, tenants, billing interval, and growth using sourced public plan data.',
      href: '/tools/printnode-cost-calculator',
      live: true
    },
    {
      label: 'Architecture',
      title: 'Piqae vs QZ Tray',
      body: 'Understand the difference between browser-local hardware access and durable remote print infrastructure.',
      href: '/compare/qz-tray',
      live: false
    },
    {
      label: 'Print management',
      title: 'Piqae vs ezeep',
      body: 'Compare API-first infrastructure with a user and endpoint-oriented cloud print platform.',
      href: '/compare/ezeep',
      live: false
    }
  ];
</script>

<Seo
  title="Compare Piqae — Printing APIs and infrastructure"
  description="Factual product comparisons, a PrintNode migration guide, and a sourced cost calculator for teams evaluating printing infrastructure."
  path="/compare"
/>

<MarketingShell>
  <section class="m-page-hero">
    <div class="m-container">
      <span class="m-eyebrow">Compare</span>
      <h1 class="m-title">Find the printing platform that fits how you build.</h1>
      <p class="m-lede">
        Compare control, deployment, pricing, migration effort, and product maturity with sourced
        guides designed to help you make a confident choice.
      </p>
    </div>
  </section>

  <section class="m-section-compact">
    <div class="m-container guide-grid">
      {#each guides as guide}
        <a class="m-card" href={guide.href}>
          <div><span>{guide.label}</span>{#if !guide.live}<small>Draft · noindex</small>{/if}</div>
          <h2>{guide.title}</h2>
          <p>{guide.body}</p>
          <strong>Read guide →</strong>
        </a>
      {/each}
    </div>
  </section>

  <section class="method m-section">
    <div class="m-container method-grid">
      <div>
        <span class="m-eyebrow">Our comparison standard</span>
        <h2 class="m-heading">Useful enough to disagree with.</h2>
      </div>
      <div>
        <p>We use official primary sources for competitor facts and show when they were checked.</p>
        <p>We include reasons to choose the other product, not only reasons to choose Piqae.</p>
        <p>Numeric claims expire after 90 days and stay out of search until they are reviewed.</p>
        <p>“Spooler accepted” never becomes a claim that a document physically printed.</p>
      </div>
    </div>
  </section>
</MarketingShell>

<style>
  .guide-grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 12px; }
  .guide-grid a { min-height: 270px; display: flex; flex-direction: column; }
  .guide-grid div { display: flex; justify-content: space-between; }
  .guide-grid div > span { color: var(--m-violet-dark); font-size: 11px; font-weight: 700; text-transform: uppercase; }
  .guide-grid small { color: var(--m-faint); }
  .guide-grid h2 { margin-top: 65px; font-size: 24px; }
  .guide-grid strong { margin-top: auto; padding-top: 22px; color: var(--m-violet-dark); font-size: 13px; }
  .method { background: #eeece6; }
  .method-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 80px; }
  .method-grid p { padding: 0 0 18px; margin: 0 0 18px; border-bottom: 1px solid var(--m-border); color: var(--m-muted); }
  @media (max-width: 700px) {
    .guide-grid, .method-grid { grid-template-columns: 1fr; }
  }
</style>

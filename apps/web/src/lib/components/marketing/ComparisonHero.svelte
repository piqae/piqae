<script lang="ts">
  let {
    eyebrow,
    title,
    description,
    verified,
    source
  }: {
    eyebrow: string;
    title: string;
    description: string;
    verified: string;
    source: string;
  } = $props();
</script>

<section class="m-page-hero">
  <div class="m-container">
    <span class="m-eyebrow">{eyebrow}</span>
    <h1 class="m-title">{title}</h1>
    <p class="m-lede">{description}</p>
    <div class="m-actions">
      <a class="m-button primary" href="/start?plan=free&source=comparison">Start with Spool</a>
      <a class="m-button" href="/tools/printnode-cost-calculator">Compare costs</a>
    </div>
    <p class="source">Last verified {verified} · <a href={source}>Official source</a></p>
  </div>
</section>

<style>
  .source { margin: 30px 0 0; color: var(--m-faint); font-size: 12px; }
  .source a { text-decoration: underline; text-underline-offset: 2px; }
</style>


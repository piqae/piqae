<script lang="ts">
  import ComparisonHero from '$lib/components/marketing/ComparisonHero.svelte';
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
</script>

<Seo
  title="Piqae vs QZ Tray — Draft architecture comparison"
  description="A draft comparison of durable remote printing infrastructure and browser-to-local hardware access."
  path="/compare/qz-tray"
  noindex
/>

<MarketingShell announcement="Draft comparison — excluded from search until hands-on and legal review">
  <ComparisonHero
    eyebrow="Draft · Piqae vs QZ Tray"
    title="Remote print infrastructure and browser-local hardware access solve different layers."
    description="Piqae coordinates durable jobs across a control plane and native agents. QZ Tray focuses on connecting web applications to local printers and other hardware."
    verified="29 July 2026"
    source="https://qz.io/"
  />
  <section class="m-section-compact">
    <div class="m-container">
      <div class="m-table-wrap">
        <table class="m-table">
          <thead><tr><th>Question</th><th>Piqae</th><th>QZ Tray</th></tr></thead>
          <tbody>
            <tr><td>Primary layer</td><td>Durable API, control plane, and remote agent</td><td>Browser-to-local hardware bridge</td></tr>
            <tr><td>Open-source posture</td><td>Apache-2.0 stack</td><td>LGPL project with commercial signing/support options</td></tr>
            <tr><td>Queue ownership</td><td>Durable server and local queues</td><td>Application-controlled local interaction</td></tr>
            <tr><td>Choose it when</td><td>Jobs must cross networks and survive restarts</td><td>A browser needs direct access to nearby hardware</td></tr>
          </tbody>
        </table>
      </div>
      <p class="m-source">Draft facts sourced from <a href="https://qz.io/">qz.io</a>. Do not index until reviewer evidence is recorded.</p>
    </div>
  </section>
</MarketingShell>

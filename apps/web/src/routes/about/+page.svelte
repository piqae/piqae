<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
</script>

<Seo
  title="Why we built Piqae — Reliable printing from the web"
  description="Piqae grew from the need to print product labels, shipping labels, and operational work reliably."
  path="/about"
/>

<MarketingShell>
  <section class="m-page-hero">
    <div class="m-container">
      <span class="m-eyebrow">Why we built Piqae</span>
      <h1 class="m-title">Built where every label matters.</h1>
      <p class="m-lede">
        Piqae began in a busy coffee operation, where product labels, shipping labels, and daily
        production depend on software reaching the right printer at the right moment.
      </p>
    </div>
  </section>

  <section class="story-visual m-container">
    <div role="img" aria-label="Reserved for product-label printing photography">
      <strong>P</strong>
      <span>Product and label photography will publish here through Payload.</span>
    </div>
    <div role="img" aria-label="Reserved for printing workflow photography">
      <span>Printer workflow</span>
    </div>
  </section>

  <section class="m-section">
    <div class="m-narrow m-prose">
      <p class="drop">
        Great operational software should make the physical work feel effortless. Printing
        connects orders and production systems to the labels people use all day.
      </p>
      <h2>Made for the reality behind the screen.</h2>
      <p>
        Real sites have changing networks, computers, drivers, and queues. Piqae grew into a
        dependable bridge that can recover cleanly, avoid careless duplicates, and show people
        when a job needs attention.
      </p>
      <p>
        Installed drivers remain authoritative because the printer computer already knows its
        hardware better than a remote catalog. Self-hosting remains first-class because not every
        print path belongs in somebody else’s cloud.
      </p>
      <h2>Open source keeps the choice with you.</h2>
      <p>
        Piqae is Apache-2.0. Teams can inspect the software at the most sensitive part of the path,
        run the control plane themselves, and keep a credible exit from the managed service.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="/start?plan=free&source=about">Start free</a>
        <a class="m-button" href="/open-source">Explore the source</a>
      </div>
      <p class="asset-note">
        Publication note: real operational images and any quantitative claims remain unpublished until
        rights, privacy, and source evidence are recorded in the CMS.
      </p>
    </div>
  </section>
</MarketingShell>

<style>
  .story-visual { display: grid; grid-template-columns: 1.35fr .65fr; gap: 10px; }
  .story-visual div {
    min-height: 510px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: 30px;
    border-radius: 20px;
    background:
      linear-gradient(160deg, transparent, rgb(23 22 27 / .72)),
      repeating-linear-gradient(45deg, #c5b69d, #c5b69d 16px, #b9aa91 16px, #b9aa91 32px);
    color: white;
  }
  .story-visual div:nth-child(2) {
    background:
      linear-gradient(160deg, transparent, rgb(23 22 27 / .76)),
      repeating-linear-gradient(-45deg, #8f8b83, #8f8b83 14px, #837f77 14px, #837f77 28px);
  }
  .story-visual strong { font-size: 86px; letter-spacing: -.09em; }
  .story-visual span { max-width: 420px; color: rgb(255 255 255 / .78); }
  .drop { color: var(--m-ink); font-size: 24px; line-height: 1.55; letter-spacing: -.02em; }
  .asset-note { margin-top: 55px !important; color: var(--m-faint) !important; font-size: 12px; }
  @media (max-width: 700px) {
    .story-visual { grid-template-columns: 1fr; }
    .story-visual div { min-height: 360px; }
  }
</style>

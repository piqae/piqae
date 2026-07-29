<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();
</script>

<Seo
  title={`Draft preview — ${data.title || data.slug}`}
  description="Private, expiring Payload CMS draft preview."
  path="/preview/cms"
  noindex
/>

<MarketingShell announcement="Private CMS preview · link expires after ten minutes">
  <section class="m-page-hero">
    <div class="m-narrow">
      <span class="m-eyebrow">{data.collection.replace('-', ' ')}</span>
      <h1 class="m-title">{data.title || data.slug}</h1>
      {#if data.summary}<p class="m-lede">{data.summary}</p>{/if}
    </div>
  </section>

  <section class="m-section-compact">
    <div class="m-narrow preview">
      {#each data.blocks as block}
        <article class="m-card">
          {#if block.eyebrow}<span class="m-eyebrow">{block.eyebrow}</span>{/if}
          {#if block.heading}<h2>{block.heading}</h2>{/if}
          {#if block.body}<p>{block.body}</p>{/if}
          {#if block.items}
            <div class="items">
              {#each block.items as item}
                <div><strong>{item.title}</strong><p>{item.body}</p></div>
              {/each}
            </div>
          {/if}
          {#if block.label && block.href}<a class="m-button" href={block.href}>{block.label}</a>{/if}
        </article>
      {/each}
    </div>
  </section>
</MarketingShell>

<style>
  .preview { display: grid; gap: 14px; }
  .preview h2 { font-size: 28px; }
  .items { display: grid; gap: 12px; margin-top: 20px; }
  .items div { padding-top: 12px; border-top: 1px solid var(--m-border); }
  .m-button { margin-top: 20px; }
</style>

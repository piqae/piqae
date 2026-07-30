<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import { docs } from '$lib/docs-content';
  const featured = docs.filter((doc) =>
    ['quickstart', 'platform-accounts', 'integration-models'].includes(doc.slug)
  );
  const firstPrint = `const customer = await piqae.accounts.getOrCreate('org_01JQ8K8M6Q', {
  name: 'Northwind Foods'
});

await customer.printPdf({
  printerId: 'prt_01K...',
  title: 'Order 481 label',
  pdf: await readFile('./label.pdf'),
  idempotencyKey: 'northwind-order-481-label-v1'
});`;
</script>

<svelte:head>
  <title>Documentation · Piqae</title>
  <meta name="description" content="Build reliable local and remote printing with Piqae." />
</svelte:head>

<div class="home">
  <header>
    <span>Piqae documentation</span>
    <h1>Printing infrastructure<br />without the mystery.</h1>
    <p>
      Add reliable local and remote printing to your product with one small SDK, durable queues,
      installed drivers, and honest live status.
    </p>
    <div>
      <a class="button primary" href="/docs/quickstart">Start printing <Icon name="arrow-right" size={13} /></a>
      <a class="button" href="/docs/printnode-migration">Migrate from PrintNode</a>
    </div>
  </header>

  <section class="code-sample">
    <div class="code-top"><span>First customer print</span><code>TypeScript</code></div>
    <pre><code>{firstPrint}</code></pre>
  </section>

  <section class="features">
    {#each featured as doc}
      <a href={`/docs/${doc.slug}`}>
        <span class="feature-icon">
          <Icon name={doc.slug === 'quickstart' ? 'bolt' : doc.slug === 'platform-accounts' ? 'agents' : 'api'} size={16} />
        </span>
        <strong>{doc.title}</strong>
        <p>{doc.description}</p>
        <span class="learn">Read guide <Icon name="arrow-right" size={11} /></span>
      </a>
    {/each}
  </section>
</div>

<style>
  .home {
    width: min(100%, 930px);
    padding: 50px 42px 80px;
  }

  .home > header > span {
    color: var(--accent-hover);
    font-size: 10px;
    font-weight: 550;
    letter-spacing: 0.02em;
  }

  h1 {
    margin: 10px 0 0;
    font-size: clamp(34px, 5vw, 50px);
    line-height: 1.08;
    font-weight: 600;
    letter-spacing: -0.05em;
  }

  header > p {
    max-width: 600px;
    margin: 15px 0 0;
    color: var(--text-secondary);
    font-size: 14px;
    line-height: 23px;
  }

  header > div {
    display: flex;
    gap: 8px;
    margin-top: 22px;
  }

  .code-sample {
    overflow: hidden;
    margin-top: 45px;
    background: var(--sidebar);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-overlay);
    box-shadow: 0 16px 50px rgb(0 0 0 / 0.12);
  }

  .code-top {
    height: 38px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 14px;
    color: var(--text-secondary);
    background: var(--surface);
    border-bottom: 1px solid var(--border-subtle);
    font-size: 10px;
  }

  .code-top code {
    color: var(--text-tertiary);
    font: 9px var(--font-mono);
  }

  pre {
    overflow-x: auto;
    margin: 0;
    padding: 20px;
    color: var(--text-secondary);
    font: 11px/19px var(--font-mono);
  }

  .features {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 10px;
    margin-top: 36px;
  }

  .features > a {
    min-height: 185px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: 16px;
    background: var(--surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
    transition:
      background-color 100ms ease,
      border-color 100ms ease;
  }

  .features > a:hover {
    background: var(--surface-hover);
    border-color: var(--border-default);
  }

  .feature-icon {
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    color: var(--accent-hover);
    background: var(--accent-soft);
    border-radius: 8px;
  }

  .features strong {
    margin-top: 13px;
    font-size: 12px;
    font-weight: 550;
  }

  .features p {
    margin: 5px 0 13px;
    color: var(--text-tertiary);
    font-size: 10px;
    line-height: 16px;
  }

  .learn {
    display: flex;
    align-items: center;
    gap: 5px;
    margin-top: auto;
    color: var(--text-secondary);
    font-size: 9px;
  }

  @media (max-width: 800px) {
    .features {
      grid-template-columns: 1fr;
    }

    .features > a {
      min-height: 150px;
    }
  }

  @media (max-width: 600px) {
    .home {
      padding: 34px 18px 60px;
    }

    h1 {
      font-size: 34px;
    }

    header > div {
      align-items: stretch;
      flex-direction: column;
    }

    pre {
      font-size: 9px;
      line-height: 16px;
    }
  }
</style>

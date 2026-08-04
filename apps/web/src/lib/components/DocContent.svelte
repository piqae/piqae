<script lang="ts">
  import type { Doc } from '$lib/docs-content';
  import Icon from './Icon.svelte';
  let { doc }: { doc: Doc } = $props();
  let copied = $state<number | null>(null);

  async function copy(code: string, index: number) {
    await navigator.clipboard.writeText(code);
    copied = index;
    setTimeout(() => (copied = null), 1200);
  }
</script>

<article>
  <header>
    <span>{doc.group}</span>
    <h1>{doc.title}</h1>
    <p>{doc.description}</p>
  </header>

  {#each doc.blocks as block, index}
    <section>
      {#if block.heading}<h2>{block.heading}</h2>{/if}
      {#if block.body}<p>{block.body}</p>{/if}
      {#if block.bullets}
        <ul>
          {#each block.bullets as bullet}<li>{bullet}</li>{/each}
        </ul>
      {/if}
      {#if block.code}
        <div class="code-block">
          <div class="code-header">
            <span>{block.language ?? 'text'}</span>
            <button onclick={() => copy(block.code ?? '', index)}>
              <Icon name={copied === index ? 'check' : 'copy'} size={11} />
              {copied === index ? 'Copied' : 'Copy'}
            </button>
          </div>
          <pre><code>{block.code}</code></pre>
        </div>
      {/if}
      {#if block.callout}
        <aside class={block.callout.tone}>
          <Icon name={block.callout.tone === 'warning' ? 'warning' : 'bolt'} size={15} />
          <div><strong>{block.callout.title}</strong><p>{block.callout.body}</p></div>
        </aside>
      {/if}
    </section>
  {/each}

  <footer>
    <p>Was this page useful?</p>
    <div><button>Yes</button><button>No</button></div>
  </footer>
</article>

<style>
  article {
    width: min(100%, 760px);
    padding: 18px 36px 80px;
  }

  header {
    padding: 22px 0 27px;
    border-bottom: 1px solid var(--border-subtle);
  }

  header span {
    color: var(--accent-hover);
    font-size: var(--text-compact);
    font-weight: 550;
    letter-spacing: 0.02em;
  }

  h1 {
    margin: 6px 0 0;
    font-size: 28px;
    line-height: 36px;
    font-weight: 600;
    letter-spacing: -0.035em;
  }

  header p {
    max-width: 620px;
    margin: 8px 0 0;
    color: var(--text-secondary);
    font-size: 13px;
    line-height: 21px;
  }

  section {
    padding-top: 25px;
  }

  h2 {
    margin: 0 0 7px;
    font-size: 16px;
    line-height: 23px;
    font-weight: 570;
    letter-spacing: -0.02em;
  }

  section > p {
    margin: 0;
    color: var(--text-secondary);
    font-size: 12px;
    line-height: 20px;
  }

  ul {
    display: grid;
    gap: 7px;
    margin: 9px 0 0;
    padding-left: 19px;
    color: var(--text-secondary);
    font-size: 12px;
    line-height: 19px;
  }

  li::marker {
    color: var(--text-tertiary);
  }

  .code-block {
    overflow: hidden;
    margin-top: 12px;
    background: var(--sidebar);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-lg);
  }

  .code-header {
    height: 34px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 9px 0 13px;
    color: var(--text-tertiary);
    background: var(--surface);
    border-bottom: 1px solid var(--border-subtle);
    font: var(--text-meta) var(--font-mono);
    text-transform: uppercase;
  }

  .code-header button {
    height: 24px;
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--text-tertiary);
    background: transparent;
    border: 0;
    border-radius: var(--radius-sm);
    font: var(--text-meta) Inter, sans-serif;
    cursor: pointer;
  }

  .code-header button:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  pre {
    overflow-x: auto;
    margin: 0;
    padding: 14px;
    color: var(--text-secondary);
    font: var(--text-compact)/var(--text-code-line) var(--font-mono);
    tab-size: 2;
  }

  aside {
    display: grid;
    grid-template-columns: 23px 1fr;
    gap: 8px;
    margin-top: 12px;
    padding: 11px 12px;
    color: var(--info);
    background: var(--info-soft);
    border: 1px solid color-mix(in oklch, var(--info), transparent 72%);
    border-radius: var(--radius-md);
  }

  aside.warning {
    color: var(--warning);
    background: var(--warning-soft);
    border-color: color-mix(in oklch, var(--warning), transparent 72%);
  }

  aside strong {
    color: var(--text-primary);
    font-size: var(--text-compact);
    font-weight: 550;
  }

  aside p {
    margin: 2px 0 0;
    color: var(--text-secondary);
    font-size: var(--text-compact);
    line-height: 16px;
  }

  article > footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: 38px;
    padding-top: 18px;
    border-top: 1px solid var(--border-subtle);
  }

  footer p {
    color: var(--text-secondary);
    font-size: var(--text-compact);
  }

  footer div {
    display: flex;
    gap: 5px;
  }

  footer button {
    height: 26px;
    padding: 0 9px;
    color: var(--text-secondary);
    background: var(--surface);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-md);
    font-size: var(--text-meta);
  }

  @media (max-width: 760px) {
    article {
      padding: 10px 18px 60px;
    }

    h1 {
      font-size: 23px;
      line-height: 31px;
    }
  }
</style>

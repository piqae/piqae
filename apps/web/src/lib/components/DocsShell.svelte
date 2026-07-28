<script lang="ts">
  import { page } from '$app/state';
  import { docs } from '$lib/docs-content';
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let { children }: { children: Snippet } = $props();
  let open = $state(false);
  const groups = $derived([...new Set(docs.map((doc) => doc.group))]);
</script>

<div class="docs-shell">
  <header class="topbar">
    <a class="brand" href="/dashboard">
      <span class="logo"><Icon name="printers" size={14} strokeWidth={2} /></span>
      <strong>Spool</strong>
      <span class="divider"></span>
      <span>Docs</span>
    </a>
    <label class="search">
      <Icon name="search" size={13} />
      <input placeholder="Search documentation…" aria-label="Search documentation" />
      <kbd>⌘ K</kbd>
    </label>
    <nav>
      <a href="/dashboard">Dashboard</a>
      <a href="https://github.com/C4CoffeeCo/spool">GitHub <Icon name="external" size={11} /></a>
    </nav>
    <button class="menu" onclick={() => (open = !open)} aria-label="Toggle documentation menu">
      <Icon name="menu" size={16} />
    </button>
  </header>

  <aside class:open>
    <a class:active={page.url.pathname === '/docs'} class="overview" href="/docs" onclick={() => (open = false)}>
      Overview
    </a>
    {#each groups as group}
      <section>
        <h2>{group}</h2>
        {#each docs.filter((doc) => doc.group === group) as doc}
          <a
            class:active={page.url.pathname === `/docs/${doc.slug}`}
            href={`/docs/${doc.slug}`}
            onclick={() => (open = false)}
          >
            {doc.title}
          </a>
        {/each}
      </section>
    {/each}
    <footer>
      <span>Spool v0.1</span>
      <a href="/docs/quickstart">API status <span class="dot"></span></a>
    </footer>
  </aside>

  <main>{@render children()}</main>
</div>

<style>
  .docs-shell {
    min-height: 100vh;
    display: grid;
    grid-template: 49px 1fr / 235px 1fr;
  }

  .topbar {
    position: sticky;
    top: 0;
    z-index: 40;
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: 235px minmax(240px, 430px) 1fr;
    align-items: center;
    gap: 24px;
    padding: 0 16px 0 13px;
    background: color-mix(in oklch, var(--canvas), transparent 6%);
    border-bottom: 1px solid var(--border-subtle);
    backdrop-filter: blur(14px);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
  }

  .brand strong {
    font-weight: 580;
  }

  .logo {
    width: 23px;
    height: 23px;
    display: grid;
    place-items: center;
    color: white;
    background: var(--accent);
    border-radius: 6px;
  }

  .divider {
    width: 1px;
    height: 16px;
    margin: 0 2px;
    background: var(--border-default);
  }

  .brand > span:last-child {
    color: var(--text-secondary);
  }

  .search {
    height: 29px;
    display: grid;
    grid-template-columns: 15px 1fr auto;
    align-items: center;
    gap: 6px;
    padding: 0 6px 0 8px;
    color: var(--text-tertiary);
    background: var(--surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
  }

  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  .search input {
    min-width: 0;
    color: var(--text-primary);
    background: transparent;
    border: 0;
    outline: 0;
    font-size: 10px;
  }

  kbd {
    padding: 2px 5px;
    color: var(--text-tertiary);
    background: var(--surface-raised);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    font: 8px var(--font-mono);
  }

  .topbar nav {
    display: flex;
    justify-content: flex-end;
    gap: 18px;
  }

  .topbar nav a {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--text-secondary);
    font-size: 10px;
  }

  .topbar nav a:hover {
    color: var(--text-primary);
  }

  .menu {
    display: none;
  }

  aside {
    position: fixed;
    inset: 49px auto 0 0;
    z-index: 20;
    width: 235px;
    overflow-y: auto;
    padding: 14px 12px 58px;
    background: var(--sidebar);
    border-right: 1px solid var(--border-subtle);
  }

  aside section {
    display: grid;
    gap: 1px;
    margin-top: 17px;
  }

  aside h2 {
    margin: 0 0 5px;
    padding: 0 8px;
    color: var(--text-tertiary);
    font-size: 8px;
    line-height: 18px;
    font-weight: 550;
    letter-spacing: 0.045em;
    text-transform: uppercase;
  }

  aside a {
    min-height: 27px;
    display: flex;
    align-items: center;
    padding: 0 8px;
    color: var(--text-tertiary);
    border-radius: var(--radius-md);
    font-size: 10px;
  }

  aside a:hover {
    color: var(--text-secondary);
    background: var(--surface-hover);
  }

  aside a.active {
    color: var(--text-primary);
    background: var(--surface-selected);
  }

  .overview {
    font-weight: 500;
  }

  aside footer {
    position: fixed;
    bottom: 0;
    left: 0;
    width: 235px;
    min-height: 43px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px;
    color: var(--text-tertiary);
    background: var(--sidebar);
    border-top: 1px solid var(--border-subtle);
    font-size: 8px;
  }

  aside footer a {
    gap: 6px;
    padding: 0;
    font-size: 8px;
  }

  .dot {
    width: 5px;
    height: 5px;
    padding: 0;
    background: var(--success);
    border-radius: 50%;
  }

  main {
    min-width: 0;
    grid-column: 2;
  }

  @media (max-width: 760px) {
    .docs-shell {
      display: block;
      padding-top: 49px;
    }

    .topbar {
      position: fixed;
      inset: 0 0 auto;
      grid-template-columns: 1fr 32px;
    }

    .search,
    .topbar nav {
      display: none;
    }

    .menu {
      width: 29px;
      height: 29px;
      display: grid;
      place-items: center;
      color: var(--text-secondary);
      background: transparent;
      border: 0;
      border-radius: var(--radius-md);
    }

    aside {
      width: 245px;
      transform: translateX(-100%);
      transition: transform 130ms ease;
      box-shadow: var(--shadow-overlay);
    }

    aside.open {
      transform: translateX(0);
    }

    aside footer {
      width: 245px;
    }
  }
</style>

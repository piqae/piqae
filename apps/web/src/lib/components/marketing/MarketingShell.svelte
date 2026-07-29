<script lang="ts">
  import type { Snippet } from 'svelte';
  import AnalyticsConsent from './AnalyticsConsent.svelte';
  import Logo from './Logo.svelte';

  let {
    children,
    announcement
  }: {
    children: Snippet;
    announcement?: string;
  } = $props();

  let open = $state(false);
  const year = new Date().getFullYear();
  const contactHref = import.meta.env.PUBLIC_SALES_EMAIL
    ? `mailto:${import.meta.env.PUBLIC_SALES_EMAIL}`
    : '/start?plan=pro&interval=annual&source=footer-contact';

  const nav = [
    { label: 'Product', href: '/how-it-works' },
    { label: 'Compare', href: '/compare' },
    { label: 'Pricing', href: '/pricing' },
    { label: 'Downloads', href: '/downloads' },
    { label: 'Docs', href: '/docs' },
    { label: 'Open source', href: '/open-source' }
  ];

  function closeOnEscape(event: KeyboardEvent) {
    if (event.key === 'Escape') open = false;
  }
</script>

<svelte:window onkeydown={closeOnEscape} />

<div class="marketing">
  <a class="skip-link" href="#main-content">Skip to content</a>
  {#if announcement}
    <a class="announcement" href="/downloads">{announcement}<span aria-hidden="true">→</span></a>
  {/if}
  <header class="site-header">
    <div class="header-inner">
      <a class="brand" href="/" aria-label="Piqae home"><Logo /></a>
      <nav id="primary-navigation" class:open aria-label="Primary navigation">
        {#each nav as item}
          <a href={item.href} onclick={() => (open = false)}>{item.label}</a>
        {/each}
        <div class="mobile-actions">
          <a href="/auth/login?return_to=%2Fdashboard">Log in</a>
          <a class="m-button primary small" href="/start?plan=free&source=nav">Start free</a>
        </div>
      </nav>
      <div class="desktop-actions">
        <a class="login" href="/auth/login?return_to=%2Fdashboard">Log in</a>
        <a class="m-button primary small" href="/start?plan=free&source=nav">Start free</a>
      </div>
      <button
        class="menu"
        type="button"
        aria-label={open ? 'Close navigation' : 'Open navigation'}
        aria-expanded={open}
        aria-controls="primary-navigation"
        onclick={() => (open = !open)}
      >
        <span></span><span></span>
      </button>
    </div>
  </header>

  <main id="main-content" tabindex="-1">{@render children()}</main>

  <footer class="site-footer">
    <div class="footer-lead m-container">
      <div>
        <a class="brand footer-brand" href="/"><Logo /></a>
        <p>Reliable printing, built into your product.</p>
      </div>
      <a class="m-button primary" href="/start?plan=free&source=footer">Start free <span>→</span></a>
    </div>
    <div class="footer-links m-container">
      <div>
        <strong>Product</strong>
        <a href="/how-it-works">How it works</a>
        <a href="/pricing">Pricing</a>
        <a href="/downloads">Downloads</a>
        <a href="/security">Security</a>
      </div>
      <div>
        <strong>Developers</strong>
        <a href="/docs">Documentation</a>
        <a href="/docs/quickstart">Quickstart</a>
        <a href="/open-source">Open source</a>
        <a href="https://github.com/C4CoffeeCo/piqae">GitHub</a>
      </div>
      <div>
        <strong>Compare</strong>
        <a href="/compare/printnode">Piqae vs PrintNode</a>
        <a href="/alternatives/printnode">PrintNode alternatives</a>
        <a href="/migrate/printnode">Migration guide</a>
        <a href="/tools/printnode-cost-calculator">Cost calculator</a>
      </div>
      <div>
        <strong>Company</strong>
        <a href="/about">Why we built Piqae</a>
        <a href="/security">Trust and security</a>
        <a href={contactHref}>Contact</a>
      </div>
    </div>
    <div class="footer-bottom m-container">
      <span>© {year} Piqae.</span>
      <span>Apache-2.0 · Built in Ōtautahi Christchurch, New Zealand</span>
    </div>
  </footer>
  <AnalyticsConsent />
</div>

<style>
  .skip-link {
    position: fixed;
    top: 10px;
    left: 10px;
    z-index: 100;
    padding: 10px 14px;
    border-radius: 8px;
    background: var(--m-dark);
    color: white;
    font-size: 13px;
    font-weight: 650;
    transform: translateY(-160%);
  }
  .skip-link:focus { transform: translateY(0); }
  .announcement {
    min-height: 34px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 6px 16px;
    background: var(--m-dark);
    color: #cac7d1;
    font-size: 12px;
  }
  .announcement:hover { color: white; }
  .site-header {
    position: sticky;
    top: 0;
    z-index: 50;
    border-bottom: 1px solid var(--m-border);
    background: rgb(247 246 242 / 0.88);
    backdrop-filter: blur(18px);
  }
  .header-inner {
    width: min(1240px, calc(100% - 40px));
    min-height: 66px;
    display: flex;
    align-items: center;
    gap: 30px;
    margin: 0 auto;
  }
  .brand {
    display: inline-flex;
    align-items: center;
    gap: 9px;
    color: var(--m-ink);
  }
  nav {
    display: flex;
    align-items: center;
    gap: 25px;
  }
  nav > a,
  .login {
    color: var(--m-muted);
    font-size: 13px;
    font-weight: 570;
    transition: color 180ms ease;
  }
  nav > a:hover,
  .login:hover { color: var(--m-ink); }
  .desktop-actions {
    display: flex;
    align-items: center;
    gap: 18px;
    margin-left: auto;
  }
  .mobile-actions,
  .menu { display: none; }
  .site-footer {
    padding: 76px 0 24px;
    border-top: 1px solid var(--m-border);
    background: #efede7;
  }
  .footer-lead {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    padding-bottom: 60px;
  }
  .footer-brand { margin-bottom: 10px; }
  .footer-lead p {
    margin: 0;
    color: var(--m-muted);
  }
  .footer-links {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 28px;
    padding: 38px 0;
    border-top: 1px solid var(--m-border);
    border-bottom: 1px solid var(--m-border);
  }
  .footer-links div { display: grid; align-content: start; gap: 9px; }
  .footer-links strong {
    margin-bottom: 4px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .footer-links a {
    color: var(--m-muted);
    font-size: 13px;
  }
  .footer-links a:hover { color: var(--m-ink); }
  .footer-bottom {
    display: flex;
    justify-content: space-between;
    gap: 20px;
    padding-top: 23px;
    color: var(--m-faint);
    font-size: 11px;
  }
  @media (max-width: 920px) {
    .header-inner { min-height: 60px; }
    .desktop-actions { display: none; }
    .menu {
      width: 38px;
      height: 38px;
      display: grid;
      place-content: center;
      gap: 5px;
      margin-left: auto;
      border: 1px solid var(--m-border);
      border-radius: 9px;
      background: transparent;
    }
    .menu span {
      width: 16px;
      height: 1px;
      background: var(--m-ink);
    }
    nav {
      position: absolute;
      top: 60px;
      left: 0;
      right: 0;
      display: none;
      align-items: stretch;
      padding: 14px 20px 20px;
      border-bottom: 1px solid var(--m-border);
      background: var(--m-canvas);
      box-shadow: 0 18px 40px rgb(23 22 27 / 0.1);
    }
    nav.open { display: grid; gap: 0; }
    nav > a {
      padding: 12px 4px;
      border-bottom: 1px solid var(--m-border);
      font-size: 15px;
    }
    .mobile-actions {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding-top: 16px;
    }
  }
  @media (max-width: 680px) {
    .header-inner { width: calc(100% - 28px); }
    .footer-lead { gap: 30px; flex-direction: column; }
    .footer-links { grid-template-columns: repeat(2, 1fr); }
    .footer-bottom { flex-direction: column; }
  }
</style>

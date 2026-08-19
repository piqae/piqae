<script lang="ts">
  import { page } from '$app/state';
  import { invalidateAll } from '$app/navigation';
  import { onMount } from 'svelte';
  import type { Snippet } from 'svelte';
  import { dashboardNavigation } from '$lib/dashboard-navigation';
  import type { DashboardMeta } from '$lib/view-types';
  import Icon from './Icon.svelte';

  let {
    mode,
    meta,
    children
  }: { mode: 'live' | 'demo'; meta: DashboardMeta; children: Snippet } = $props();
  let interactive = $state(false);
  let theme = $state<'dark' | 'light'>('dark');

  const nav = $derived(dashboardNavigation(meta));

  function isActive(href: string): boolean {
    return href === '/dashboard'
      ? page.url.pathname === href || page.url.pathname.startsWith('/dashboard/local')
      : page.url.pathname === href || page.url.pathname.startsWith(`${href}/`);
  }

  function toggleTheme() {
    theme = theme === 'dark' ? 'light' : 'dark';
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
    localStorage.setItem('piqae-theme', theme);
  }

  onMount(() => {
    interactive = true;
    theme = document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
    if (mode !== 'live') return;
    const source = new EventSource('/api/events');
    let refreshTimer: ReturnType<typeof setTimeout> | undefined;
    const scheduleRefresh = () => {
      if (refreshTimer) return;
      refreshTimer = setTimeout(() => {
        refreshTimer = undefined;
        void invalidateAll();
      }, 500);
    };
    for (const eventName of [
      'job.updated',
      'agent.enrolled',
      'agent.updated',
      'printer.updated',
      'webhook.created',
      'webhook.deleted',
      'resync_required'
    ]) {
      source.addEventListener(eventName, scheduleRefresh);
    }
    source.addEventListener('open', scheduleRefresh);
    return () => {
      source.close();
      if (refreshTimer) clearTimeout(refreshTimer);
    };
  });
</script>

<svelte:head>
  <script>
    try {
      const saved = localStorage.getItem('piqae-theme') ?? localStorage.getItem('spool-theme');
      if (saved === 'light' || saved === 'dark') {
        document.documentElement.dataset.theme = saved;
        document.documentElement.style.colorScheme = saved;
        localStorage.setItem('piqae-theme', saved);
        localStorage.removeItem('spool-theme');
      }
    } catch {}
  </script>
</svelte:head>

<div class="shell">
  <header class="topbar">
    <a class="brand" href="/dashboard" aria-label="Piqae operations">
      <span class="logo"><Icon name="printers" size={14} strokeWidth={1.9} /></span>
      <strong>Piqae</strong>
    </a>

    <nav aria-label="Main navigation">
      {#each nav as item}
        <a
          href={item.href}
          class:active={isActive(item.href)}
          aria-current={isActive(item.href) ? 'page' : undefined}
        >
          <Icon name={item.icon} size={14} />
          <span>{item.label}</span>
        </a>
      {/each}
    </nav>

    <div class="utility">
      <span class="service" title={`Piqae ${meta.deployment.replace('_', ' ')} · v${meta.version}`}>
        <span class="service-dot" aria-hidden="true"></span>
        <span class="deployment">{meta.deployment.replace('_', ' ')}</span>
        <small class="mono">v{meta.version}</small>
      </span>
      <button
        class="icon-button"
        onclick={toggleTheme}
        aria-label="Toggle color theme"
        disabled={!interactive}
      >
        <Icon name={theme === 'dark' ? 'sun' : 'moon'} size={14} />
      </button>
      {#if mode === 'live'}
        <a class="sign-out" href="/auth/logout?return_to=/login">Sign out</a>
      {/if}
    </div>
  </header>

  {#if mode === 'demo'}
    <div class="demo-banner" role="status">
      <Icon name="warning" size={13} />
      Demo data — no control-plane requests are being made.
    </div>
  {/if}

  <main>{@render children()}</main>
</div>

<style>
  .shell {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
  }

  .topbar {
    position: sticky;
    top: 0;
    z-index: 30;
    height: var(--topbar-height);
    display: flex;
    align-items: center;
    gap: 20px;
    padding: 0 20px;
    background: var(--sidebar);
    border-bottom: 1px solid var(--border-subtle);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    flex: 0 0 auto;
  }

  .logo {
    width: 24px;
    height: 24px;
    display: inline-grid;
    place-items: center;
    color: white;
    background: var(--accent);
    border-radius: 6px;
    box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.12);
  }

  .brand strong {
    font-family: var(--font-display);
    font-size: var(--text-section);
    font-weight: 560;
    letter-spacing: -0.018em;
  }

  nav {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
  }

  nav a {
    height: var(--control-compact);
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 10px;
    color: var(--text-secondary);
    border-radius: var(--radius-md);
    font-size: var(--text-compact);
    font-weight: 500;
    transition:
      color 90ms ease,
      background-color 90ms ease;
  }

  nav a:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  nav a.active {
    color: var(--text-primary);
    background: var(--surface-selected);
  }

  nav a :global(svg) {
    color: var(--icon-muted);
  }

  nav a.active :global(svg) {
    color: var(--text-secondary);
  }

  .utility {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .service {
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--text-tertiary);
    font-size: var(--text-meta);
  }

  .service-dot {
    width: 6px;
    height: 6px;
    flex: 0 0 auto;
    background: var(--success);
    border-radius: 50%;
    box-shadow: 0 0 0 2px var(--success-soft);
  }

  .deployment {
    text-transform: capitalize;
  }

  .service small {
    color: var(--text-tertiary);
    font-size: var(--text-meta);
  }

  .sign-out {
    height: var(--control-compact);
    display: inline-flex;
    align-items: center;
    padding: 0 10px;
    color: var(--text-secondary);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    font-size: var(--text-compact);
    font-weight: 500;
  }

  .sign-out:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .demo-banner {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    padding: 7px 14px;
    color: var(--warning);
    background: var(--warning-soft);
    border-bottom: 1px solid color-mix(in oklch, var(--warning), transparent 78%);
    font-size: var(--text-meta);
    font-weight: 500;
  }

  main {
    width: min(100%, 1400px);
    max-width: 100%;
    min-width: 0;
    flex: 1;
    margin: 0 auto;
    padding: 20px 24px 56px;
  }

  @media (max-width: 720px) {
    .topbar {
      gap: 12px;
      padding: 0 12px;
    }

    .brand strong,
    .service {
      display: none;
    }

    nav a span {
      display: none;
    }

    nav a {
      padding: 0 9px;
    }

    main {
      padding: 16px 14px 40px;
    }
  }
</style>

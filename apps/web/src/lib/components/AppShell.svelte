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
  let sidebarOpen = $state(false);
  let interactive = $state(false);
  let theme = $state<'dark' | 'light'>('dark');

  const nav = $derived(dashboardNavigation(meta));

  const utility = [
    { href: '/dashboard/settings', label: 'Settings', icon: 'settings' }
  ] as const;

  function isActive(href: string): boolean {
    if (
      href === '/dashboard/nodes' &&
      ['/dashboard/local', '/dashboard/agents'].some((route) =>
        page.url.pathname.startsWith(route)
      )
    ) {
      return true;
    }
    if (
      href === '/dashboard/developers' &&
      ['/dashboard/api-keys', '/dashboard/webhooks'].some((route) =>
        page.url.pathname.startsWith(route)
      )
    ) {
      return true;
    }
    return href === '/dashboard'
      ? page.url.pathname === href
      : page.url.pathname.startsWith(`${href}/`) || page.url.pathname === href;
  }

  function toggleTheme() {
    theme = theme === 'dark' ? 'light' : 'dark';
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
    localStorage.setItem('piqae-theme', theme);
  }

  function closeSidebar() {
    sidebarOpen = false;
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
  <aside id="primary-sidebar" class:open={sidebarOpen}>
    <div class="workspace">
      <a
        class="workspace-switch"
        href="/dashboard"
        onclick={closeSidebar}
        aria-label="Piqae overview"
        title={meta.auth.workspaceSwitching
          ? 'Piqae workspace'
          : 'This deployment has one workspace'}
      >
        <span class="logo"><Icon name="printers" size={14} strokeWidth={1.9} /></span>
        <span class="workspace-name">
          <strong>Piqae</strong>
          <small>Printing workspace</small>
        </span>
        <span class="deployment">{meta.deployment.replace('_', ' ')}</span>
      </a>
    </div>

    <nav aria-label="Main navigation">
      <div class="nav-group">
        <span class="nav-label">Workspace</span>
        {#each nav as item}
          <a
            href={item.href}
            class:active={isActive(item.href)}
            aria-current={isActive(item.href) ? 'page' : undefined}
            onclick={closeSidebar}
          >
            <Icon name={item.icon} size={14} />
            <span>{item.label}</span>
          </a>
        {/each}
      </div>

      <div class="nav-group utility">
        {#each utility as item}
          <a
            href={item.href}
            class:active={isActive(item.href)}
            aria-current={isActive(item.href) ? 'page' : undefined}
            onclick={closeSidebar}
          >
            <Icon name={item.icon} size={14} />
            <span>{item.label}</span>
          </a>
        {/each}
      </div>
    </nav>

    <div class="sidebar-footer">
      <div class="service">
        <span class="service-dot" aria-hidden="true"></span>
        <div>
          <strong>{meta.deployment.replace('_', ' ')}</strong>
          <small>v{meta.version}</small>
        </div>
      </div>
      <button class="theme-button" onclick={toggleTheme} aria-label="Toggle color theme">
        <Icon name={theme === 'dark' ? 'sun' : 'moon'} size={14} />
      </button>
    </div>
  </aside>

  {#if sidebarOpen}
    <button class="scrim" onclick={closeSidebar} aria-label="Close navigation"></button>
  {/if}

  <section class="main">
    <div class="mobile-bar">
      <button
        onclick={() => (sidebarOpen = true)}
        aria-label="Open navigation"
        aria-expanded={sidebarOpen}
        aria-controls="primary-sidebar"
        disabled={!interactive}
      >
        <Icon name="menu" size={17} />
      </button>
      <a class="mobile-brand" href="/dashboard">
        <span class="logo"><Icon name="printers" size={13} strokeWidth={2} /></span>
        Piqae
      </a>
      <span></span>
    </div>
    {#if mode === 'demo'}
      <div class="demo-banner" role="status">
        <Icon name="warning" size={12} />
        Demo data — no control-plane requests are being made.
      </div>
    {/if}
    <main>{@render children()}</main>
  </section>
</div>

<style>
  .shell {
    min-height: 100vh;
    display: grid;
    grid-template-columns: 218px minmax(0, 1fr);
  }

  aside {
    position: fixed;
    inset: 0 auto 0 0;
    z-index: 30;
    width: 218px;
    display: flex;
    flex-direction: column;
    background: var(--sidebar);
    border-right: 1px solid var(--border-subtle);
  }

  .workspace {
    padding: 9px 8px 7px;
  }

  .logo {
    width: 26px;
    height: 26px;
    display: inline-grid;
    flex: 0 0 auto;
    place-items: center;
    color: white;
    background: var(--accent);
    border-radius: 7px;
    box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.12);
  }

  .workspace-switch {
    width: 100%;
    min-height: 42px;
    display: grid;
    grid-template-columns: 26px minmax(0, 1fr) auto;
    align-items: center;
    gap: 8px;
    padding: 4px 7px;
    color: var(--text-secondary);
    text-align: left;
    background: transparent;
    border: 0;
    border-radius: var(--radius-md);
    cursor: pointer;
  }

  .workspace-switch:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .workspace-name {
    display: grid;
    min-width: 0;
    line-height: 15px;
  }

  .workspace-name strong {
    overflow: hidden;
    color: var(--text-primary);
    font-family: var(--font-display);
    font-size: 13px;
    font-weight: 560;
    letter-spacing: -0.018em;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .workspace-name small {
    color: var(--text-tertiary);
    font-size: 9px;
  }

  .deployment {
    padding: 2px 5px;
    color: var(--text-tertiary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: 4px;
    font-size: 8px;
    line-height: 13px;
    text-transform: capitalize;
  }

  nav {
    flex: 1;
    display: flex;
    flex-direction: column;
    padding: 6px 8px;
  }

  .nav-group {
    display: grid;
    gap: 1px;
  }

  .nav-group.utility {
    margin-top: auto;
  }

  nav a {
    height: 30px;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 0 8px;
    color: var(--text-tertiary);
    border-radius: var(--radius-md);
    font-size: 12px;
    font-weight: 470;
    transition:
      color 90ms ease,
      background-color 90ms ease;
  }

  nav a:hover {
    color: var(--text-secondary);
    background: var(--surface-hover);
  }

  nav a.active {
    color: var(--text-primary);
    background: var(--surface-selected);
    box-shadow: inset 0 0 0 1px var(--border-subtle);
  }

  nav a :global(svg) {
    color: var(--icon-muted);
  }

  nav a.active :global(svg) {
    color: var(--text-secondary);
  }

  .nav-label {
    height: 25px;
    display: flex;
    align-items: end;
    padding: 0 8px 5px;
    color: var(--text-tertiary);
    font-size: 9px;
    font-weight: 520;
    letter-spacing: 0.035em;
    text-transform: uppercase;
  }

  .sidebar-footer {
    min-height: 55px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-subtle);
  }

  .service {
    min-width: 0;
    flex: 1;
    display: flex;
    align-items: center;
    gap: 7px;
  }

  .service-dot {
    width: 6px;
    height: 6px;
    flex: 0 0 auto;
    background: var(--success);
    border-radius: 50%;
    box-shadow: 0 0 0 2px var(--success-soft);
  }

  .service div {
    min-width: 0;
    display: grid;
  }

  .service strong {
    overflow: hidden;
    color: var(--text-secondary);
    font-size: 10px;
    font-weight: 500;
    line-height: 14px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .service small {
    color: var(--text-tertiary);
    font-size: 9px;
    line-height: 13px;
  }

  .theme-button,
  .mobile-bar button {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    flex: 0 0 auto;
    color: var(--text-tertiary);
    background: transparent;
    border: 0;
    border-radius: var(--radius-md);
    cursor: pointer;
  }

  .theme-button:hover,
  .mobile-bar button:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .main {
    min-width: 0;
    width: 100%;
    max-width: 100vw;
    grid-column: 2;
    overflow-x: clip;
  }

  main {
    width: min(100%, 1500px);
    max-width: 100%;
    min-width: 0;
    min-height: 100vh;
    margin: 0 auto;
    padding: 22px 28px 56px;
  }

  .mobile-bar {
    display: none;
  }

  .demo-banner {
    min-height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 5px 12px;
    color: var(--warning);
    background: var(--warning-soft);
    border-bottom: 1px solid color-mix(in oklch, var(--warning), transparent 78%);
    font-size: 9px;
    font-weight: 500;
  }

  .scrim {
    display: none;
  }

  @media (max-width: 760px) {
    .shell {
      display: block;
    }

    aside {
      width: 228px;
      transform: translateX(-100%);
      transition: transform 140ms ease;
      box-shadow: var(--shadow-overlay);
    }

    aside.open {
      transform: translateX(0);
    }

    .scrim {
      position: fixed;
      inset: 0;
      z-index: 20;
      display: block;
      background: rgb(0 0 0 / 0.45);
      border: 0;
    }

    .main {
      min-height: 100vh;
    }

    .mobile-bar {
      height: 45px;
      display: grid;
      grid-template-columns: 32px 1fr 32px;
      align-items: center;
      padding: 0 10px;
      background: var(--sidebar);
      border-bottom: 1px solid var(--border-subtle);
    }

    .mobile-brand {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 7px;
      font-weight: 560;
    }

    .mobile-brand .logo {
      width: 21px;
      height: 21px;
    }

    main {
      min-height: calc(100vh - 45px);
      padding: 18px 14px 40px;
    }
  }
</style>

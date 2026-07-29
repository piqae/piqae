<script lang="ts">
  import { page } from '$app/state';
  import { invalidateAll } from '$app/navigation';
  import { onMount } from 'svelte';
  import type { Snippet } from 'svelte';
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

  const nav = [
    { href: '/dashboard', label: 'Overview', icon: 'activity' },
    { href: '/dashboard/jobs', label: 'Jobs', icon: 'jobs' },
    { href: '/dashboard/printers', label: 'Printers', icon: 'printers' },
    { href: '/dashboard/nodes', label: 'Nodes', icon: 'agents' },
    { href: '/dashboard/developers', label: 'Developers', icon: 'api' }
  ] as const;

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
    localStorage.setItem('spool-theme', theme);
  }

  function closeSidebar() {
    sidebarOpen = false;
  }

  onMount(() => {
    interactive = true;
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
      const saved = localStorage.getItem('spool-theme');
      if (saved === 'light' || saved === 'dark') {
        document.documentElement.dataset.theme = saved;
        document.documentElement.style.colorScheme = saved;
      }
    } catch {}
  </script>
</svelte:head>

<div class="shell">
  <aside id="primary-sidebar" class:open={sidebarOpen}>
    <div class="workspace">
      <a class="brand" href="/dashboard" onclick={closeSidebar} aria-label="Spool overview">
        <span class="logo"><Icon name="printers" size={14} strokeWidth={2} /></span>
        <span>Spool</span>
      </a>
      <button
        class="workspace-switch"
        aria-label="Current workspace"
        disabled
        title={meta.auth.workspaceSwitching
          ? 'Workspace switching is available in the account menu'
          : 'This deployment has one workspace'}
      >
        <span class="avatar">SP</span>
        <span class="workspace-name">
          <strong>Spool</strong>
          <small>{meta.deployment.replace('_', ' ')}</small>
        </span>
        <Icon name="chevron-down" size={13} />
      </button>
    </div>

    <nav aria-label="Main navigation">
      <div class="nav-group">
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
        Spool
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
    padding: 13px 10px 7px;
  }

  .brand {
    height: 30px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 6px;
    font-size: 14px;
    font-weight: 580;
    letter-spacing: -0.02em;
  }

  .logo {
    width: 23px;
    height: 23px;
    display: inline-grid;
    flex: 0 0 auto;
    place-items: center;
    color: white;
    background: var(--accent);
    border-radius: 6px;
    box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.12);
  }

  .workspace-switch {
    width: 100%;
    min-height: 41px;
    display: grid;
    grid-template-columns: 26px minmax(0, 1fr) 14px;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
    padding: 4px 6px;
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

  .avatar {
    width: 25px;
    height: 25px;
    display: grid;
    place-items: center;
    color: var(--text-primary);
    background: var(--surface-raised);
    border: 1px solid var(--border-default);
    border-radius: 6px;
    font-size: 9px;
    font-weight: 600;
  }

  .workspace-name {
    display: grid;
    min-width: 0;
    line-height: 15px;
  }

  .workspace-name strong {
    overflow: hidden;
    color: var(--text-primary);
    font-size: 12px;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .workspace-name small {
    color: var(--text-tertiary);
    font-size: 10px;
  }

  nav {
    flex: 1;
    display: flex;
    flex-direction: column;
    padding: 6px 8px;
  }

  .nav-group {
    display: grid;
    gap: 2px;
  }

  .nav-group.utility {
    margin-top: auto;
  }

  nav a {
    height: 29px;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 0 8px;
    color: var(--text-tertiary);
    border-radius: var(--radius-md);
    font-size: 12px;
    font-weight: 450;
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
  }

  nav a :global(svg) {
    color: var(--icon-muted);
  }

  nav a.active :global(svg) {
    color: var(--text-secondary);
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
    align-items: flex-start;
    gap: 7px;
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

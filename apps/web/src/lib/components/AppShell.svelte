<script lang="ts">
  import { page } from '$app/state';
  import { invalidateAll } from '$app/navigation';
  import { onMount } from 'svelte';
  import type { Snippet } from 'svelte';
  import { dashboardNavigation } from '$lib/dashboard-navigation';
  import { settleDashboardRefresh } from '$lib/dashboard-refresh';
  import type { DashboardMeta } from '$lib/view-types';
  import Icon from './Icon.svelte';

  let {
    mode,
    meta,
    viewer,
    workspaces,
    children
  }: {
    mode: 'live' | 'demo';
    meta: DashboardMeta;
    viewer: {
      email: string;
      name: string | null;
      organizationId: string;
      role: string | null;
    } | null;
    workspaces: Array<{
      organizationId: string;
      organizationName: string;
      role: string;
    }>;
    children: Snippet;
  } = $props();
  let interactive = $state(false);
  let theme = $state<'dark' | 'light'>('dark');

  const nav = $derived(dashboardNavigation(meta));
  const currentWorkspace = $derived(
    workspaces.find((workspace) => workspace.organizationId === viewer?.organizationId) ?? null
  );
  const displayName = $derived(viewer?.name || viewer?.email || 'Account');
  const initials = $derived(
    displayName
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((part) => part[0]?.toUpperCase())
      .join('') || 'P'
  );

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
    let refreshInFlight = false;
    const scheduleRefresh = () => {
      if (refreshTimer || refreshInFlight) return;
      refreshTimer = setTimeout(() => {
        refreshTimer = undefined;
        refreshInFlight = true;
        void settleDashboardRefresh(invalidateAll()).finally(() => {
          refreshInFlight = false;
        });
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
    <details class="account-switcher">
      <summary aria-label={`Account and workspace: ${currentWorkspace?.organizationName ?? 'Piqae'}`}>
        <span class="avatar" aria-hidden="true">{initials}</span>
        <span class="account-label">
          <strong>{currentWorkspace?.organizationName ?? 'Piqae'}</strong>
          <small>{workspaces.length > 1 ? `${workspaces.length} workspaces` : 'Workspace'}</small>
        </span>
        <Icon name="chevron-down" size={13} />
      </summary>
      <div class="account-menu">
        <div class="viewer">
          <span class="avatar large" aria-hidden="true">{initials}</span>
          <span>
            <strong>{displayName}</strong>
            <small>{viewer?.email}</small>
          </span>
        </div>
        <div class="menu-section" aria-label="Workspaces">
          <p>Workspaces</p>
          {#each workspaces as workspace}
            {#if workspace.organizationId === viewer?.organizationId}
              <div class="workspace current" aria-current="true">
                <span class="workspace-mark">{workspace.organizationName.slice(0, 1).toUpperCase()}</span>
                <span><strong>{workspace.organizationName}</strong><small>{workspace.role} · Current</small></span>
                <Icon name="check" size={13} />
              </div>
            {:else}
              <form method="POST" action="/auth/switch">
                <input type="hidden" name="organization_id" value={workspace.organizationId} />
                <input type="hidden" name="return_to" value={page.url.pathname} />
                <button class="workspace" type="submit">
                  <span class="workspace-mark">{workspace.organizationName.slice(0, 1).toUpperCase()}</span>
                  <span><strong>{workspace.organizationName}</strong><small>{workspace.role}</small></span>
                </button>
              </form>
            {/if}
          {/each}
          {#if meta.auth.workspaceSwitching}
            <a class="menu-link" href="/onboarding"><Icon name="plus" size={13} /> Create workspace</a>
          {/if}
        </div>
        <div class="menu-actions">
          <a class="menu-link" href="/dashboard/settings"><Icon name="settings" size={13} /> Settings</a>
          <a class="menu-link" href="/auth/logout?return_to=/login"><Icon name="logout" size={13} /> Sign out</a>
        </div>
      </div>
    </details>

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

  .account-switcher {
    position: relative;
    flex: 0 0 auto;
  }

  .account-switcher summary {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 190px;
    padding: 5px 8px 5px 5px;
    border-radius: var(--radius-md);
    cursor: pointer;
    list-style: none;
  }

  .account-switcher summary::-webkit-details-marker { display: none; }
  .account-switcher summary:hover,
  .account-switcher[open] summary { background: var(--surface-hover); }

  .avatar,
  .workspace-mark {
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    flex: 0 0 auto;
    color: white;
    background: var(--accent);
    border-radius: 7px;
    font-size: 10px;
    font-weight: 700;
  }

  .avatar.large { width: 34px; height: 34px; border-radius: 9px; font-size: 12px; }

  .account-label,
  .viewer span,
  .workspace span:not(.workspace-mark) {
    display: grid;
    min-width: 0;
  }

  .account-label { flex: 1; }
  .account-label strong { font-size: var(--text-compact); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .account-label small,
  .viewer small,
  .workspace small { color: var(--text-tertiary); font-size: var(--text-meta); }

  .account-menu {
    position: absolute;
    top: calc(100% + 8px);
    left: 0;
    z-index: 50;
    width: 300px;
    padding: 8px;
    color: var(--text-primary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
    box-shadow: 0 16px 40px rgb(0 0 0 / 0.3);
  }

  .viewer { display: flex; align-items: center; gap: 10px; padding: 8px; }
  .viewer strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--text-compact); }
  .menu-section,
  .menu-actions { padding-top: 7px; margin-top: 7px; border-top: 1px solid var(--border-subtle); }
  .menu-section > p { margin: 3px 8px 6px; color: var(--text-tertiary); font-size: var(--text-meta); font-weight: 600; text-transform: uppercase; letter-spacing: .06em; }
  .workspace,
  .menu-link {
    width: 100%;
    min-height: 38px;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 6px 8px;
    color: var(--text-secondary);
    background: transparent;
    border: 0;
    border-radius: var(--radius-md);
    text-align: left;
    font: inherit;
  }
  button.workspace { cursor: pointer; }
  .workspace:hover,
  .menu-link:hover { color: var(--text-primary); background: var(--surface-hover); }
  .workspace.current { color: var(--text-primary); }
  .workspace > span:nth-child(2) { flex: 1; }
  .workspace strong { font-size: var(--text-compact); font-weight: 550; }
  .workspace-mark { width: 24px; height: 24px; background: var(--surface-selected); color: var(--text-secondary); }

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

    .account-label,
    .service {
      display: none;
    }

    .account-switcher summary { min-width: auto; padding-right: 5px; }
    .account-menu { width: min(300px, calc(100vw - 24px)); }

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

<script lang="ts">
  import { afterNavigate } from '$app/navigation';
  import {
    captureMarketingEvent,
    initializeMarketingAnalytics,
    stopMarketingAnalytics
  } from '$lib/marketing/analytics';
  import { onMount } from 'svelte';

  type Choice = 'granted' | 'denied' | null;
  let choice = $state<Choice>(null);
  let initialized = false;
  const storageKey = 'spool_analytics_consent_v1';

  async function start() {
    if (initialized || !import.meta.env.PUBLIC_POSTHOG_KEY) return;
    await initializeMarketingAnalytics(
      import.meta.env.PUBLIC_POSTHOG_KEY,
      import.meta.env.PUBLIC_POSTHOG_HOST || 'https://eu.i.posthog.com'
    );
    initialized = true;
    capturePage();
  }

  function capturePage() {
    if (!initialized) return;
    const path = window.location.pathname;
    captureMarketingEvent('marketing_page_view', { path });
    if (
      path.startsWith('/compare/') ||
      path.startsWith('/alternatives/') ||
      path.startsWith('/migrate/')
    ) {
      captureMarketingEvent('comparison_viewed', { path });
    }
  }

  function decide(next: Exclude<Choice, null>) {
    choice = next;
    localStorage.setItem(storageKey, next);
    if (next === 'granted') start();
    else stopMarketingAnalytics();
  }

  onMount(() => {
    const stored = localStorage.getItem(storageKey);
    choice = stored === 'granted' || stored === 'denied' ? stored : null;
    if (choice === 'granted') start();

    const click = (event: MouseEvent) => {
      if (!initialized) return;
      const target = event.target instanceof Element ? event.target.closest('a') : null;
      if (!(target instanceof HTMLAnchorElement)) return;
      const destination = target.getAttribute('href') ?? '';
      const isDownload = target.hasAttribute('data-marketing-download');
      if (!target.classList.contains('m-button') && !destination.startsWith('/start') && !isDownload)
        return;
      captureMarketingEvent('cta_clicked', {
        path: window.location.pathname,
        destination: destination.slice(0, 160),
        label: (target.textContent ?? '').trim().slice(0, 80)
      });
      if (destination.startsWith('/start')) {
        captureMarketingEvent('signup_started', {
          path: window.location.pathname,
          destination: destination.slice(0, 160)
        });
      }
      if (isDownload) {
        captureMarketingEvent('download_selected', {
          path: window.location.pathname,
          platform: target.dataset.platform ?? 'unknown'
        });
      }
    };
    document.addEventListener('click', click);
    return () => document.removeEventListener('click', click);
  });

  afterNavigate(() => capturePage());
</script>

{#if choice === null && import.meta.env.PUBLIC_POSTHOG_KEY}
  <aside class="consent" aria-label="Analytics preference">
    <div>
      <strong>Help improve Spool</strong>
      <p>
        Allow anonymous product-marketing analytics. Session replay is off and print data is never
        collected. <a href="/security#analytics">Details</a>
      </p>
    </div>
    <div class="actions">
      <button type="button" onclick={() => decide('denied')}>No thanks</button>
      <button class="accept" type="button" onclick={() => decide('granted')}>Allow analytics</button>
    </div>
  </aside>
{/if}

<style>
  .consent {
    position: fixed;
    z-index: 100;
    right: 18px;
    bottom: 18px;
    width: min(430px, calc(100% - 36px));
    display: flex;
    align-items: center;
    gap: 20px;
    padding: 16px;
    border: 1px solid rgb(255 255 255 / .12);
    border-radius: 14px;
    background: #17161b;
    color: white;
    box-shadow: 0 20px 60px rgb(23 22 27 / .28);
  }
  .consent > div:first-child { flex: 1; }
  strong { font-size: 12px; }
  p { margin: 3px 0 0; color: #aaa8b1; font-size: 10px; line-height: 1.45; }
  p a { color: white; text-decoration: underline; text-underline-offset: 2px; }
  .actions { flex: none; display: grid; gap: 5px; }
  button {
    min-height: 30px;
    padding: 0 10px;
    border: 1px solid rgb(255 255 255 / .12);
    border-radius: 7px;
    background: transparent;
    color: #c8c6ce;
    cursor: pointer;
    font-size: 10px;
  }
  button.accept { border-color: #006aff; background: #006aff; color: white; }
  @media (max-width: 520px) {
    .consent { align-items: stretch; flex-direction: column; }
    .actions { grid-template-columns: 1fr 1fr; }
  }
</style>

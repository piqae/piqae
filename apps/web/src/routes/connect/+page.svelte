<script lang="ts">
  import { onMount } from 'svelte';
  import { consumeNodeConnectFragment, nativeNodeConnectUrl } from '$lib/node-connect-fragment';

  let nativeUrl = $state<string | null>(null);

  onMount(() => {
    const invitation = consumeNodeConnectFragment(window.location, (url) =>
      window.history.replaceState({}, '', url)
    );
    nativeUrl = invitation
      ? nativeNodeConnectUrl(invitation.enrolmentToken, invitation.controlPlaneUrl)
      : null;
  });

  function openPiqae() {
    if (nativeUrl) window.location.assign(nativeUrl);
  }
</script>

<svelte:head>
  <title>Open Piqae</title>
  <meta name="robots" content="noindex, nofollow" />
</svelte:head>

<main aria-live="polite">
  <section>
    <span>Piqae node connection</span>
    {#if nativeUrl}
      <h1>Connect this printer computer</h1>
      <p>
        Piqae will show the service requesting access and let you choose exactly which local
        printers it may use.
      </p>
      <button type="button" onclick={openPiqae}>Open Piqae</button>
      <a href={`/downloads#${nativeUrl.split('#')[1]}`}>Piqae is not installed</a>
      <small>This invitation expires shortly and can be accepted only once.</small>
    {:else}
      <h1>This connection link is invalid or has already been cleared</h1>
      <p>Return to the service that asked you to connect this printer computer and try again.</p>
    {/if}
    <noscript>JavaScript is required to complete this secure connection handoff.</noscript>
  </section>
</main>

<style>
  main { min-height: 100vh; display: grid; place-items: center; padding: 24px; background: var(--canvas); }
  section { width: min(100%, 430px); padding: 28px; background: var(--surface); border: 1px solid var(--border-default); border-radius: var(--radius-overlay); box-shadow: var(--shadow-overlay); }
  span { color: var(--accent); font-size: 10px; font-weight: 650; letter-spacing: .08em; text-transform: uppercase; }
  h1 { margin: 10px 0 0; color: var(--text-primary); font-size: 24px; letter-spacing: -.035em; }
  p, small { color: var(--text-secondary); line-height: 1.55; }
  button, a { display: flex; min-height: 40px; align-items: center; justify-content: center; margin-top: 18px; border-radius: 8px; font: inherit; }
  button { width: 100%; color: white; background: var(--accent); border: 0; cursor: pointer; }
  a { color: var(--text-primary); border: 1px solid var(--border-default); }
  small { display: block; margin-top: 16px; font-size: 11px; }
</style>

<script lang="ts">
  import { onMount } from 'svelte';

  onMount(() => {
    // If macOS did not open the installed app, retain the fragment only across
    // this same-origin fallback navigation. Fragments are never sent to the
    // server in either request.
    window.location.replace(`/downloads${window.location.hash}`);
  });
</script>

<svelte:head>
  <title>Open Piqae</title>
  <meta name="robots" content="noindex, nofollow" />
</svelte:head>

<main aria-live="polite">
  <p>Opening Piqae…</p>
  <noscript>JavaScript is required to complete this secure connection handoff.</noscript>
</main>

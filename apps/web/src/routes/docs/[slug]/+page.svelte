<script lang="ts">
  import DocContent from '$lib/components/DocContent.svelte';
  let { data } = $props();
</script>

<svelte:head>
  <title>{data.doc.title} · Spool Docs</title>
  <meta name="description" content={data.doc.description} />
</svelte:head>

<DocContent doc={data.doc} />

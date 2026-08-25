<script lang="ts">
  import { relativeLabel } from '$lib/elapsed';

  let { value }: { value: string } = $props();

  const format = (input: string): string => relativeLabel(Date.now() - new Date(input).getTime());
</script>

<time datetime={value} title={new Date(value).toLocaleString()}>{format(value)}</time>

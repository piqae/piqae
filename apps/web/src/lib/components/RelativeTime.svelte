<script lang="ts">
  let { value }: { value: string } = $props();

  function format(input: string): string {
    const delta = Date.now() - new Date(input).getTime();
    const minutes = Math.max(0, Math.round(delta / 60_000));
    if (minutes < 1) return 'now';
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.round(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.round(hours / 24);
    return `${days}d ago`;
  }
</script>

<time datetime={value} title={new Date(value).toLocaleString()}>{format(value)}</time>

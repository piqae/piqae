<script lang="ts">
  export type SegmentedOption = {
    value: string;
    label: string;
  };

  let {
    value = $bindable(''),
    label,
    options
  }: {
    value?: string;
    label: string;
    options: SegmentedOption[];
  } = $props();
</script>

<div class="ui-segmented" aria-label={label}>
  {#each options as option}
    <button
      type="button"
      aria-pressed={value === option.value}
      onclick={() => (value = option.value)}
    >
      {option.label}
    </button>
  {/each}
</div>

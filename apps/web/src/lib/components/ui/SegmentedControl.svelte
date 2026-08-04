<script lang="ts">
  export type SegmentedOption = {
    value: string;
    label: string;
  };

  let {
    value = $bindable(''),
    label,
    options,
    onchange
  }: {
    value?: string;
    label: string;
    options: SegmentedOption[];
    /** Supplied when the selection is owned elsewhere, e.g. the query string. */
    onchange?: (value: string) => void;
  } = $props();

  function select(next: string) {
    if (onchange) onchange(next);
    else value = next;
  }
</script>

<div class="ui-segmented" role="group" aria-label={label}>
  {#each options as option}
    <button type="button" aria-pressed={value === option.value} onclick={() => select(option.value)}>
      {option.label}
    </button>
  {/each}
</div>

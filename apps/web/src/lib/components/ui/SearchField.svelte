<script lang="ts">
  import Icon from '../Icon.svelte';

  let {
    value = $bindable(''),
    label = 'Search',
    placeholder = 'Search…'
  }: {
    value?: string;
    label?: string;
    placeholder?: string;
  } = $props();
</script>

<label class="ui-search">
  <span class="sr-only">{label}</span>
  <Icon name="search" size={13} />
  <input bind:value aria-label={label} {placeholder} />
</label>

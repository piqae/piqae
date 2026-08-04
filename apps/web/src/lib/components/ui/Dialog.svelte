<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    open = $bindable(false),
    title,
    description,
    labelledBy,
    children,
    footer,
    onclose
  }: {
    open?: boolean;
    title: string;
    description?: string;
    labelledBy: string;
    children: Snippet;
    footer?: Snippet;
    onclose?: () => void;
  } = $props();

  let element = $state<HTMLDialogElement>();

  // Keep the native modal in step with the bound `open` prop so callers only
  // ever toggle state, never reach for showModal()/close() themselves.
  $effect(() => {
    if (!element) return;
    if (open && !element.open) element.showModal();
    if (!open && element.open) element.close();
  });

  function handleClose() {
    open = false;
    onclose?.();
  }
</script>

<dialog
  bind:this={element}
  class="ui-dialog"
  aria-labelledby={labelledBy}
  onclose={handleClose}
>
  <div class="ui-dialog__header">
    <div>
      <h2 id={labelledBy}>{title}</h2>
      {#if description}<p>{description}</p>{/if}
    </div>
    <button
      class="icon-button"
      type="button"
      aria-label={`Close ${title.toLowerCase()} dialog`}
      onclick={handleClose}
    >
      &times;
    </button>
  </div>
  {@render children()}
  {#if footer}
    <div class="ui-dialog__footer">{@render footer()}</div>
  {/if}
</dialog>

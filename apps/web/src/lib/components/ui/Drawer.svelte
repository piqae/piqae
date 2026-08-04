<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    open = false,
    title,
    eyebrow,
    labelledBy,
    children,
    actions,
    onclose
  }: {
    open?: boolean;
    title: string;
    eyebrow?: string;
    labelledBy: string;
    children: Snippet;
    actions?: Snippet;
    onclose: () => void;
  } = $props();

  let element = $state<HTMLDialogElement>();

  // Native <dialog> gives us the focus trap and Escape handling for free; the
  // drawer is only a matter of pinning it to the inline-end edge.
  $effect(() => {
    if (!element) return;
    if (open && !element.open) element.showModal();
    if (!open && element.open) element.close();
  });
</script>

<dialog bind:this={element} aria-labelledby={labelledBy} onclose={onclose}>
  <header>
    <div class="heading">
      {#if eyebrow}<span class="eyebrow">{eyebrow}</span>{/if}
      <h2 id={labelledBy}>{title}</h2>
    </div>
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
    <button class="icon-button" type="button" aria-label="Close detail" onclick={onclose}>
      &times;
    </button>
  </header>
  <div class="body">{@render children()}</div>
</dialog>

<style>
  dialog {
    width: min(560px, 100vw);
    max-width: 100vw;
    height: 100dvh;
    max-height: 100dvh;
    margin: 0 0 0 auto;
    padding: 0;
    overflow: hidden;
    color: var(--text-primary);
    background: var(--surface);
    border: 0;
    border-left: 1px solid var(--border-default);
    box-shadow: var(--shadow-overlay);
  }

  /*
   * Lay out only when open. Setting `display` unconditionally would override
   * the user-agent `dialog:not([open]) { display: none }` rule and paint the
   * closed drawer into the page.
   */
  dialog[open] {
    display: flex;
    flex-direction: column;
    animation: slide-in 180ms ease-out;
  }

  dialog::backdrop {
    background: rgb(7 9 13 / 0.5);
  }

  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px 12px 18px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .heading {
    min-width: 0;
    flex: 1;
  }

  .eyebrow {
    display: block;
    color: var(--text-tertiary);
    font-size: var(--text-meta);
    line-height: var(--text-meta-line);
  }

  h2 {
    margin: 0;
    overflow: hidden;
    font-family: var(--font-display);
    font-size: 15px;
    line-height: 22px;
    font-weight: 560;
    letter-spacing: -0.02em;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 18px;
    display: grid;
    align-content: start;
    gap: 18px;
  }

  @keyframes slide-in {
    from {
      transform: translateX(8px);
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    dialog[open] {
      animation: none;
    }
  }
</style>

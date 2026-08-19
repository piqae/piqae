<script lang="ts">
  type Name =
    | 'activity'
    | 'agents'
    | 'api'
    | 'arrow-right'
    | 'bolt'
    | 'check'
    | 'chevron-down'
    | 'clock'
    | 'copy'
    | 'docs'
    | 'external'
    | 'jobs'
    | 'logout'
    | 'menu'
    | 'moon'
    | 'more'
    | 'plus'
    | 'printers'
    | 'search'
    | 'settings'
    | 'sun'
    | 'warning'
    | 'webhooks'
    | 'x';

  let { name, size = 15, strokeWidth = 1.7 }: { name: Name; size?: number; strokeWidth?: number } =
    $props();
</script>

<svg
  aria-hidden="true"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
>
  {#if name === 'activity'}
    <path d="M3 12h4l2.2-6 4.1 12 2.1-6H21" />
  {:else if name === 'agents'}
    <rect x="4" y="3.5" width="16" height="12" rx="2" />
    <path d="M8 20h8M12 15.5V20" />
  {:else if name === 'api'}
    <path d="m8 9-4 3 4 3M16 9l4 3-4 3M14 5l-4 14" />
  {:else if name === 'arrow-right'}
    <path d="M5 12h14M14 7l5 5-5 5" />
  {:else if name === 'bolt'}
    <path d="m13 2-8 12h7l-1 8 8-12h-7z" />
  {:else if name === 'check'}
    <path d="m5 12 4 4L19 6" />
  {:else if name === 'chevron-down'}
    <path d="m7 9 5 5 5-5" />
  {:else if name === 'clock'}
    <circle cx="12" cy="12" r="9" />
    <path d="M12 7v5l3 2" />
  {:else if name === 'copy'}
    <rect x="8" y="8" width="11" height="11" rx="2" />
    <path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" />
  {:else if name === 'docs'}
    <path d="M6 3.5h9l3 3V20a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4.5a1 1 0 0 1 1-1Z" />
    <path d="M14.5 3.5V7H18M8 11h7M8 15h7" />
  {:else if name === 'external'}
    <path d="M14 4h6v6M20 4l-9 9" />
    <path d="M18 13v5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h5" />
  {:else if name === 'jobs'}
    <path d="M7 3.5h10a2 2 0 0 1 2 2v15l-3-2-4 2-4-2-3 2v-15a2 2 0 0 1 2-2Z" />
    <path d="M8.5 8h7M8.5 12h5" />
  {:else if name === 'logout'}
    <path d="M10 4H5a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h5M14 8l4 4-4 4M8 12h10" />
  {:else if name === 'menu'}
    <path d="M4 7h16M4 12h16M4 17h16" />
  {:else if name === 'moon'}
    <path d="M20 15.4A8 8 0 0 1 8.6 4a8 8 0 1 0 11.4 11.4Z" />
  {:else if name === 'more'}
    <circle cx="5" cy="12" r="1" fill="currentColor" stroke="none" />
    <circle cx="12" cy="12" r="1" fill="currentColor" stroke="none" />
    <circle cx="19" cy="12" r="1" fill="currentColor" stroke="none" />
  {:else if name === 'plus'}
    <path d="M12 5v14M5 12h14" />
  {:else if name === 'printers'}
    <path d="M7 8V3.5h10V8M7 17H5a2 2 0 0 1-2-2v-5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2h-2" />
    <path d="M7 14h10v6.5H7zM17 11h.01" />
  {:else if name === 'search'}
    <circle cx="10.5" cy="10.5" r="6.5" />
    <path d="m16 16 4 4" />
  {:else if name === 'settings'}
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H3v-4h.1a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1a1.7 1.7 0 0 0 1.9.3 1.7 1.7 0 0 0 1-1.6V3h4v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1Z" />
  {:else if name === 'sun'}
    <circle cx="12" cy="12" r="4" />
    <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
  {:else if name === 'warning'}
    <path d="M10.3 4 2.7 17.5A1.7 1.7 0 0 0 4.2 20h15.6a1.7 1.7 0 0 0 1.5-2.5L13.7 4a2 2 0 0 0-3.4 0Z" />
    <path d="M12 9v4M12 17h.01" />
  {:else if name === 'webhooks'}
    <path d="M7.5 8.5a4 4 0 1 1 6.8-3M16.5 8.5a4 4 0 1 1 2 7.5M12 16a4 4 0 1 1-5-5" />
    <circle cx="12" cy="12" r="1.5" />
  {:else if name === 'x'}
    <path d="m6 6 12 12M18 6 6 18" />
  {/if}
</svg>

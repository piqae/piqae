<script lang="ts">
  import type { DashboardJobEvent } from '$lib/view-types';

  let { events }: { events: DashboardJobEvent[] } = $props();

  // Newest first: operators read the current state before its history.
  const ordered = $derived(events.slice().reverse());
</script>

<ol>
  {#each ordered as event, index}
    <li>
      <span class="marker" class:current={index === 0}><span></span></span>
      <div class="event">
        <div class="event-head">
          <strong>{event.message}</strong>
          <time class="numeric" datetime={event.occurredAt}>
            {new Date(event.occurredAt).toLocaleTimeString([], {
              hour: '2-digit',
              minute: '2-digit',
              second: '2-digit'
            })}
          </time>
        </div>
        <p>
          <span>{event.observer.replaceAll('_', ' ')}</span>
          <span aria-hidden="true">·</span>
          <span>{event.authority.replaceAll('_', ' ')}</span>
          <span aria-hidden="true">·</span>
          <span class="mono">sequence {event.sequence}</span>
        </p>
        {#if Object.keys(event.details).length > 0}
          <pre>{JSON.stringify(event.details, null, 2)}</pre>
        {/if}
      </div>
    </li>
  {/each}
</ol>

<style>
  ol {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: 22px minmax(0, 1fr);
    gap: 10px;
  }

  .marker {
    position: relative;
    display: flex;
    justify-content: center;
  }

  .marker::after {
    position: absolute;
    inset: 18px auto 0;
    width: 1px;
    content: '';
    background: var(--border-default);
  }

  li:last-child .marker::after {
    display: none;
  }

  .marker > span {
    position: relative;
    z-index: 1;
    width: 8px;
    height: 8px;
    margin-top: 8px;
    background: var(--surface-raised);
    border: 2px solid var(--text-tertiary);
    border-radius: 50%;
  }

  .marker.current > span {
    background: var(--accent);
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }

  .event {
    padding: 0 0 18px;
  }

  li:last-child .event {
    padding-bottom: 0;
  }

  .event-head {
    display: flex;
    justify-content: space-between;
    gap: 16px;
  }

  .event-head strong {
    font-size: var(--text-compact);
    line-height: var(--text-compact-line);
    font-weight: 500;
  }

  .event-head time {
    flex: 0 0 auto;
    color: var(--text-tertiary);
    font-size: var(--text-meta);
  }

  .event p {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    margin: 3px 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-meta);
    line-height: var(--text-meta-line);
    text-transform: capitalize;
  }

  pre {
    overflow-x: auto;
    margin: 8px 0 0;
    padding: 9px 10px;
    color: var(--text-secondary);
    background: var(--canvas);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    font: var(--text-code) / var(--text-code-line) var(--font-mono);
  }
</style>

/**
 * The dashboard speaks one coarse time vocabulary everywhere: `9m`, `2h`, `3d`.
 * `RelativeTime` renders a past instant with it and duration displays such as
 * the uncertain-handoff tile render a span with it, so "updated 2h ago" and
 * "uncertain for 2h" stay comparable at a glance.
 */

const MINUTE = 60_000;

/**
 * Coarse span for `milliseconds`, or `null` when less than a minute has passed
 * and no honest bucket applies yet. Negative spans clamp to `null` rather than
 * inventing a future.
 */
export function elapsedLabel(milliseconds: number): string | null {
  if (!Number.isFinite(milliseconds)) return null;
  const minutes = Math.max(0, Math.round(milliseconds / MINUTE));
  if (minutes < 1) return null;
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.round(hours / 24)}d`;
}

/** A past instant in the same vocabulary: `now`, `9m ago`, `2h ago`. */
export function relativeLabel(milliseconds: number): string {
  const label = elapsedLabel(milliseconds);
  return label === null ? 'now' : `${label} ago`;
}

/**
 * A span for prose that cannot read as an instant. Sub-minute spans say so
 * plainly instead of borrowing `now`, which would read as "resolved".
 */
export function durationLabel(milliseconds: number): string {
  return elapsedLabel(milliseconds) ?? 'under a minute';
}

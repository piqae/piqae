import { durationLabel } from './elapsed';
import type { DashboardJob } from './view-types';

/**
 * `delivery_uncertain` means the job crossed the spooler handoff and nobody can
 * prove ink reached paper. A bare count of those jobs reads as "zero or not
 * zero": a job that turned uncertain a minute ago usually resolves itself,
 * while one that has been uncertain for two hours is a fault waiting for a
 * human. Age is what separates them, so the dashboard summarises both.
 */
export const UNCERTAIN_DELIVERY_STATE = 'delivery_uncertain';

/** Query string that narrows the operational job list to exactly these jobs. */
export const UNCERTAIN_DELIVERY_HREF = `/dashboard?view=jobs&state=${UNCERTAIN_DELIVERY_STATE}`;

export interface UncertainDeliverySummary {
  count: number;
  /**
   * Last recorded observation of the longest-unresolved uncertain job, or
   * `null` when nothing is uncertain (or no job carried a usable timestamp).
   */
  oldestObservedAt: string | null;
  /** Coarse span since that observation: `under a minute`, `9m`, `2h`, `3d`. */
  oldestLabel: string | null;
}

type TimedJob = Pick<DashboardJob, 'state' | 'createdAt' | 'updatedAt'>;

/**
 * Piqae has no dedicated "became uncertain at" stamp on the job list: the
 * public job resource carries `created_at` only, and the dashboard adapter
 * mirrors it into `updatedAt`. The last recorded observation therefore *bounds*
 * how long delivery has been unproven — it never measures it, and the wording
 * around this summary must not claim otherwise.
 */
export function summariseUncertainDelivery(
  jobs: readonly TimedJob[],
  now: number = Date.now()
): UncertainDeliverySummary {
  let count = 0;
  let oldestObservedAt: string | null = null;
  let oldest = Number.POSITIVE_INFINITY;

  for (const job of jobs) {
    if (job.state !== UNCERTAIN_DELIVERY_STATE) continue;
    count += 1;
    const observedAt = firstTimestamp(job.updatedAt, job.createdAt);
    if (observedAt === null) continue;
    const observed = new Date(observedAt).getTime();
    if (observed < oldest) {
      oldest = observed;
      oldestObservedAt = observedAt;
    }
  }

  return {
    count,
    oldestObservedAt,
    oldestLabel: oldestObservedAt === null ? null : durationLabel(now - oldest)
  };
}

function firstTimestamp(...candidates: (string | null | undefined)[]): string | null {
  for (const candidate of candidates) {
    if (typeof candidate !== 'string') continue;
    if (Number.isFinite(new Date(candidate).getTime())) return candidate;
  }
  return null;
}

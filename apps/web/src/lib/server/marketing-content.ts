import { env } from '$env/dynamic/private';
import {
  printNodeFallbackSnapshot,
  type PrintNodePricingSnapshot,
  type PrintNodeTier
} from '$lib/marketing/calculator';
import type { PricingProse } from '$lib/server/pricing';

let lastKnownPrintNodePricing: PrintNodePricingSnapshot | null = null;
let lastKnownPricingProse: PricingProse = {};

function nonNegativeInteger(value: unknown): number | null {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 ? value : null;
}

export async function loadPricingProse(fetcher: typeof fetch): Promise<PricingProse> {
  const baseUrl = env.PAYLOAD_CMS_URL?.replace(/\/$/, '');
  if (!baseUrl) return lastKnownPricingProse;
  try {
    const response = await fetcher(
      `${baseUrl}/api/pricing-display?where[_status][equals]=published&limit=2&depth=0`,
      {
        headers: env.PAYLOAD_CMS_READ_TOKEN
          ? { authorization: `users API-Key ${env.PAYLOAD_CMS_READ_TOKEN}` }
          : {}
      }
    );
    if (!response.ok) return lastKnownPricingProse;
    const body = (await response.json()) as { docs?: unknown[] };
    const next: PricingProse = {};
    for (const value of body.docs ?? []) {
      if (!value || typeof value !== 'object') continue;
      const item = value as Record<string, unknown>;
      if (
        (item.plan === 'free' || item.plan === 'pro') &&
        typeof item.headline === 'string' &&
        item.headline.trim() !== ''
      ) {
        next[item.plan] = { headline: item.headline.slice(0, 240) };
      }
    }
    lastKnownPricingProse = next;
    return next;
  } catch {
    return lastKnownPricingProse;
  }
}

function parsePrintNodeTier(value: unknown): PrintNodeTier | null {
  if (!value || typeof value !== 'object') return null;
  const tier = value as Record<string, unknown>;
  if (typeof tier.name !== 'string') return null;
  const baseMonthlyCents = nonNegativeInteger(tier.monthlyCents);
  const includedJobs = nonNegativeInteger(tier.includedJobs);
  const extraJobUnit = nonNegativeInteger(tier.extraJobUnit);
  const extraJobUnitCents = nonNegativeInteger(tier.extraJobUnitCents);
  if (
    baseMonthlyCents === null ||
    includedJobs === null ||
    extraJobUnit === null ||
    extraJobUnitCents === null
  ) {
    return null;
  }
  const optional = (item: unknown) =>
    item === null || item === undefined ? undefined : (nonNegativeInteger(item) ?? undefined);
  return {
    name: tier.name,
    baseMonthlyCents,
    annualBaseCents: optional(tier.annualCents),
    includedJobs,
    annualIncludedJobs: optional(tier.annualIncludedJobs),
    includedComputers:
      tier.includedComputers === null || tier.includedComputers === undefined
        ? null
        : nonNegativeInteger(tier.includedComputers),
    includedSubaccounts: nonNegativeInteger(tier.includedSubaccounts) ?? 0,
    extraJobUnit,
    extraJobUnitCents,
    extraSubaccountCents: optional(tier.extraSubaccountCents)
  };
}

function parsePrintNodeSnapshot(value: unknown): PrintNodePricingSnapshot | null {
  if (!value || typeof value !== 'object') return null;
  const snapshot = value as Record<string, unknown>;
  if (
    snapshot.currency !== 'USD' ||
    typeof snapshot.sourceUrl !== 'string' ||
    typeof snapshot.observedAt !== 'string' ||
    typeof snapshot.reviewDueAt !== 'string' ||
    !Array.isArray(snapshot.tiers)
  ) {
    return null;
  }
  try {
    const source = new URL(snapshot.sourceUrl);
    if (source.hostname !== 'www.printnode.com' || source.pathname !== '/en/pricing') return null;
  } catch {
    return null;
  }
  const tiers = snapshot.tiers
    .map(parsePrintNodeTier)
    .filter((tier): tier is PrintNodeTier => tier !== null);
  if (tiers.length !== snapshot.tiers.length || tiers.length < 6) return null;
  return {
    currency: 'USD',
    sourceUrl: snapshot.sourceUrl,
    observedAt: snapshot.observedAt.slice(0, 10),
    reviewDueAt: snapshot.reviewDueAt.slice(0, 10),
    tiers
  };
}

export async function loadPrintNodePricingSnapshot(
  fetcher: typeof fetch
): Promise<PrintNodePricingSnapshot> {
  const fallback = lastKnownPrintNodePricing ?? printNodeFallbackSnapshot;
  const baseUrl = env.PAYLOAD_CMS_URL?.replace(/\/$/, '');
  if (!baseUrl) return fallback;
  const query = new URLSearchParams({
    'where[_status][equals]': 'published',
    'where[label][equals]': 'PrintNode USD public pricing',
    sort: '-observedAt',
    limit: '1',
    depth: '0'
  });
  try {
    const response = await fetcher(`${baseUrl}/api/competitor-pricing-snapshots?${query}`, {
      headers: env.PAYLOAD_CMS_READ_TOKEN
        ? { authorization: `users API-Key ${env.PAYLOAD_CMS_READ_TOKEN}` }
        : {}
    });
    if (!response.ok) return fallback;
    const body = (await response.json()) as { docs?: unknown[] };
    const parsed = parsePrintNodeSnapshot(body.docs?.[0]);
    if (!parsed) return fallback;
    lastKnownPrintNodePricing = parsed;
    return parsed;
  } catch {
    return fallback;
  }
}

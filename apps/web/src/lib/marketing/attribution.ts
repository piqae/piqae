import type { BillingInterval, PlanSlug } from './types';

const plans: PlanSlug[] = ['free', 'pro'];
const intervals: BillingInterval[] = ['monthly', 'annual'];

export interface MarketingAttribution {
  plan: PlanSlug;
  interval: BillingInterval;
  source: string;
  firstTouch: Record<string, string>;
  lastTouch: Record<string, string>;
  capturedAt: string;
}

function safeValue(value: string | null, limit = 120): string {
  return (value ?? '').replace(/[^\w .:/+-]/g, '').slice(0, limit);
}

function safeReferrer(value?: string): string {
  if (!value) return '';
  try {
    const referrer = new URL(value);
    return safeValue(`${referrer.origin}${referrer.pathname}`, 200);
  } catch {
    return '';
  }
}

export function buildAttribution(
  url: URL,
  existing?: MarketingAttribution,
  referrer?: string
): MarketingAttribution {
  const planValue = url.searchParams.get('plan') as PlanSlug | null;
  const intervalValue = url.searchParams.get('interval') as BillingInterval | null;
  const touch = Object.fromEntries(
    ['utm_source', 'utm_medium', 'utm_campaign', 'utm_content', 'utm_term']
      .map((key) => [key, safeValue(url.searchParams.get(key))])
      .filter(([, value]) => value)
  );
  const sanitizedReferrer = safeReferrer(referrer);
  if (sanitizedReferrer) touch.referrer = sanitizedReferrer;
  return {
    plan: planValue && plans.includes(planValue) ? planValue : 'free',
    interval: intervalValue && intervals.includes(intervalValue) ? intervalValue : 'monthly',
    source: safeValue(url.searchParams.get('source')) || 'direct',
    firstTouch: existing?.firstTouch && Object.keys(existing.firstTouch).length > 0 ? existing.firstTouch : touch,
    lastTouch: touch,
    capturedAt: new Date().toISOString()
  };
}

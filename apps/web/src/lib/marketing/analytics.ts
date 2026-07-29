import type posthog from 'posthog-js';

let client: typeof posthog | null = null;

export async function initializeMarketingAnalytics(key: string, host: string) {
  if (client || !key) return client;
  const module = await import('posthog-js');
  client = module.default;
  client.init(key, {
    api_host: host,
    capture_pageview: false,
    capture_pageleave: false,
    disable_session_recording: true,
    autocapture: false,
    persistence: 'localStorage',
    person_profiles: 'identified_only',
    respect_dnt: true
  });
  return client;
}

export function captureMarketingEvent(
  event:
    | 'marketing_page_view'
    | 'cta_clicked'
    | 'comparison_viewed'
    | 'cost_calculator_completed'
    | 'download_selected'
    | 'signup_started'
    | 'signup_completed'
    | 'first_agent_enrolled'
    | 'first_job_spooler_accepted',
  properties: Record<string, string | number | boolean | null> = {}
) {
  client?.capture(event, properties);
}

export function stopMarketingAnalytics() {
  client?.opt_out_capturing();
  client?.reset();
  client = null;
}


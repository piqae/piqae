import type { RequestHandler } from './$types';
import type { JobOptions } from '@piqae/sdk';
import { PiqaeError } from '@piqae/sdk';
import { dashboardMode, dashboardSdk } from '$lib/server/dashboard-data';
import {
  createA4TestPdf,
  LocalAgentConfigurationError,
  localAgentError,
  localAgentRequest,
  relayLocalAgent
} from '$lib/server/local-agent';

interface LocalProfile {
  profile_id: string;
  revision: number;
  options: JobOptions;
}

function isA4Paper(value: string): boolean {
  return /^(?:(?:iso[_ -])?a4(?:[._-](?:210x297(?:mm)?|fullbleed))?|210x297(?:mm)?)$/i.test(
    value.trim()
  );
}

export const POST: RequestHandler = async (event) => {
  if (dashboardMode() !== 'live') {
    return Response.json(
      { code: 'demo_mode', message: 'Hosted test delivery is disabled in demo mode.' },
      { status: 409 }
    );
  }

  try {
    let value: unknown;
    try {
      value = await event.request.json();
    } catch {
      return Response.json(
        { code: 'invalid_request', message: 'The request body must be valid JSON.' },
        { status: 400 }
      );
    }
    if (!value || typeof value !== 'object') {
      return Response.json({ code: 'invalid_request', message: 'Select a printer and profile.' }, { status: 400 });
    }
    const input = value as Record<string, unknown>;
    const printerId = typeof input.printer_id === 'string' ? input.printer_id.trim() : '';
    const profileId = typeof input.profile_id === 'string' ? input.profile_id.trim() : '';
    if (!printerId || !profileId || input.confirmed !== true) {
      return Response.json(
        { code: 'confirmation_required', message: 'Confirm the selected printer and profile.' },
        { status: 400 }
      );
    }

    const profileResponse = await localAgentRequest(
      event.fetch,
      `/v1/local/printers/${encodeURIComponent(printerId)}/profiles`
    );
    if (!profileResponse.ok) return relayLocalAgent(profileResponse);
    const profiles = (await profileResponse.json()) as LocalProfile[];
    const profile = profiles.find((candidate) => candidate.profile_id === profileId);
    if (!profile) {
      return Response.json(
        { code: 'profile_not_found', message: 'The selected local profile no longer exists.' },
        { status: 409 }
      );
    }
    const paper = profile.options.paper;
    const a4Compatible = !paper || isA4Paper(paper);
    if (!a4Compatible) {
      return Response.json(
        {
          code: 'a4_profile_required',
          message: `The selected profile explicitly uses ${paper}; choose an A4 profile.`
        },
        { status: 409 }
      );
    }

    const client = dashboardSdk(event);
    const job = await client.jobs.create(
      {
        printer_id: printerId,
        title: 'Piqae A4 end-to-end test',
        source: 'piqae-dashboard',
        content_type: 'pdf',
        content: { type: 'base64', data: createA4TestPdf() },
        options: profile.options,
        deliveries: 1,
        expire_after_seconds: 900,
        metadata: {
          test: 'a4-end-to-end',
          profile_id: profile.profile_id,
          profile_revision: String(profile.revision)
        }
      },
      `a4-test-${crypto.randomUUID()}`
    );
    return Response.json(
      { job_id: job.id, state: job.state },
      { status: 201, headers: { 'cache-control': 'no-store, private' } }
    );
  } catch (error) {
    if (error instanceof LocalAgentConfigurationError) return localAgentError(error);
    if (error instanceof PiqaeError) {
      return Response.json(
        {
          code: error.code,
          message: error.message,
          request_id: error.requestId ?? null,
          retryable: error.retryable
        },
        { status: error.status, headers: { 'cache-control': 'no-store, private' } }
      );
    }
    return Response.json(
      {
        code: 'hosted_test_failed',
        message: 'The end-to-end test could not be registered.'
      },
      { status: 502, headers: { 'cache-control': 'no-store, private' } }
    );
  }
};

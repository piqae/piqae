import type { RequestHandler } from './$types';
import type { JobOptions } from '@spool/sdk';
import { SpoolError } from '@spool/sdk';
import { dashboardMode, dashboardSdk } from '$lib/server/dashboard-data';
import {
  createA4TestPdf,
  LocalAgentConfigurationError,
  localAgentError,
  localAgentRequest
} from '$lib/server/local-agent';

interface LocalProfile {
  profile_id: string;
  revision: number;
  options: JobOptions;
}

export const POST: RequestHandler = async (event) => {
  if (dashboardMode() !== 'live') {
    return Response.json(
      { code: 'demo_mode', message: 'Hosted test delivery is disabled in demo mode.' },
      { status: 409 }
    );
  }

  try {
    const value: unknown = await event.request.json();
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
    if (!profileResponse.ok) return await localAgentError(new Error('Profile lookup failed.'));
    const profiles = (await profileResponse.json()) as LocalProfile[];
    const profile = profiles.find((candidate) => candidate.profile_id === profileId);
    if (!profile) {
      return Response.json(
        { code: 'profile_not_found', message: 'The selected local profile no longer exists.' },
        { status: 409 }
      );
    }
    const paper = profile.options.paper;
    const a4Compatible =
      !paper ||
      /(^|[^a-z0-9])a4([^a-z0-9]|$)/i.test(paper) ||
      paper.toLowerCase().includes('iso_a4');
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
        title: 'Spool A4 end-to-end test',
        source: 'spool-dashboard',
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
    if (error instanceof SpoolError) {
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

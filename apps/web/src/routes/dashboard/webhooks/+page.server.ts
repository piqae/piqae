import { fail } from '@sveltejs/kit';
import type { Actions, PageServerLoad } from './$types';
import {
  dashboardMode,
  dashboardSdk,
  dashboardSource,
  preventSecretCaching,
  presentDashboardError
} from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => {
  try {
    const webhooks = await dashboardSource(event).api.webhooks();
    return { webhooks: webhooks.data, dataError: null };
  } catch (error) {
    return { webhooks: [], dataError: presentDashboardError(error) };
  }
};

export const actions: Actions = {
  createWebhook: async (event) => {
    preventSecretCaching(event);
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'createWebhook',
        error: { message: 'Webhook mutations are disabled while demo data is active.' }
      });
    }
    const data = await event.request.formData();
    const url = String(data.get('url') ?? '').trim();
    const events = data.getAll('events').map(String).filter(Boolean);
    try {
      const parsed = new URL(url);
      if (!['https:', 'http:'].includes(parsed.protocol)) throw new Error('invalid protocol');
    } catch {
      return fail(400, {
        mutation: 'createWebhook',
        error: { message: 'Enter a valid HTTP or HTTPS webhook URL.' }
      });
    }
    if (events.length === 0) {
      return fail(400, {
        mutation: 'createWebhook',
        error: { message: 'Select at least one event family.' }
      });
    }
    try {
      const webhook = await dashboardSdk(event).webhooks.create({ url, events });
      return {
        mutation: 'createWebhook',
        webhook: { id: webhook.id, url: webhook.url, secret: webhook.secret }
      };
    } catch (error) {
      return fail(502, {
        mutation: 'createWebhook',
        error: { message: presentDashboardError(error).message }
      });
    }
  },
  deleteWebhook: async (event) => {
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'deleteWebhook',
        error: { message: 'Webhook mutations are disabled while demo data is active.' }
      });
    }
    const data = await event.request.formData();
    const webhookId = String(data.get('webhook_id') ?? '').trim();
    if (!webhookId) {
      return fail(400, {
        mutation: 'deleteWebhook',
        error: { message: 'Webhook ID is required.' }
      });
    }
    try {
      await dashboardSdk(event).webhooks.remove(webhookId);
      return { mutation: 'deleteWebhook', deletedWebhookId: webhookId };
    } catch (error) {
      return fail(502, {
        mutation: 'deleteWebhook',
        error: { message: presentDashboardError(error).message }
      });
    }
  }
};

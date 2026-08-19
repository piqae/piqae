import { fail, redirect } from '@sveltejs/kit';
import type { Actions, PageServerLoad, RequestEvent } from './$types';
import { authMode } from '$lib/server/auth-config';
import { currentLocalIdentity, exchangeLocalOwnerCredential } from '$lib/server/local-owner-auth';
import { safeReturnTo } from '$lib/server/safe-return-to';
import { isSameOriginRequest } from '$lib/server/same-origin';
import {
  authenticatePassword,
  beginMagicAuth,
  beginTotpChallenge,
  completeEmailVerification,
  completeMagicAuth,
  completeTotp,
  isAdvancedChallenge,
  registerPassword,
  requestPasswordReset,
  resetPassword,
  saveEmailVerificationChallenge,
  saveWorkosSession
} from '$lib/server/workos-first-party-auth';

// Defence in depth for the current single-instance deployment. This cannot be
// treated as the production distributed rate-limit boundary when web replicas
// are scaled out; WorkOS/Radar remains authoritative until a shared edge or
// datastore-backed limiter is configured.
const attempts = new Map<string, { count: number; resetAt: number }>();
const WINDOW_MS = 10 * 60 * 1_000;
const MAX_ATTEMPTS = 12;

function trustedClientAddress(event: RequestEvent) {
  try {
    return event.getClientAddress();
  } catch {
    return 'unknown';
  }
}

function limited(event: RequestEvent, action: string) {
  const now = Date.now();
  const address = trustedClientAddress(event);
  const key = `${action}:${address}`;
  const entry = attempts.get(key);
  if (!entry || entry.resetAt <= now) {
    if (attempts.size >= 5_000) {
      for (const [candidate, value] of attempts) if (value.resetAt <= now) attempts.delete(candidate);
      // Keep this defence-in-depth map strictly bounded even under a flood of
      // unique addresses. WorkOS Radar remains the authoritative distributed
      // control until Piqae runs more than one web replica.
      while (attempts.size >= 5_000) {
        const oldest = attempts.keys().next().value;
        if (typeof oldest !== 'string') break;
        attempts.delete(oldest);
      }
    }
    attempts.set(key, { count: 1, resetAt: now + WINDOW_MS });
    return false;
  }
  entry.count += 1;
  return entry.count > MAX_ATTEMPTS;
}

function noStore(event: RequestEvent) {
  event.setHeaders({ 'cache-control': 'private, no-store' });
}

async function form(event: RequestEvent, action: string) {
  noStore(event);
  if (!isSameOriginRequest(event.request, event.url)) return null;
  if (limited(event, action)) return null;
  return event.request.formData();
}

function text(data: FormData, name: string, max: number) {
  const value = data.get(name);
  return typeof value === 'string' && value.length <= max ? value.trim() : '';
}

function raw(data: FormData, name: string, max: number) {
  const value = data.get(name);
  return typeof value === 'string' && value.length <= max ? value : '';
}

function target(data: FormData) {
  return safeReturnTo(text(data, 'return_to', 2_048));
}

function genericFailure(step = 'password') {
  return fail(400, { invalid: true, step });
}

export const load: PageServerLoad = async (event) => {
  noStore(event);
  const returnTo = safeReturnTo(event.url.searchParams.get('return_to'));
  if (authMode === 'demo') redirect(303, returnTo);
  if (authMode === 'local' && (await currentLocalIdentity(event))) redirect(303, returnTo);
  return {
    authMode,
    returnTo,
    resetToken: event.url.searchParams.get('token')?.slice(0, 2_048) ?? '',
    initialStep: event.url.searchParams.has('token') ? 'reset' : 'password'
  };
};

export const actions: Actions = {
  local: async (event) => {
    if (authMode !== 'local') return genericFailure();
    const data = await form(event, 'local');
    if (!data) return fail(403, { invalid: true, step: 'local' });
    const credential = text(data, 'credential', 512);
    if (credential.length < 40) return genericFailure('local');
    try {
      await exchangeLocalOwnerCredential(event, credential);
    } catch {
      return genericFailure('local');
    }
    redirect(303, target(data));
  },

  password: async (event) => {
    if (authMode !== 'workos') return genericFailure();
    const data = await form(event, 'password');
    if (!data) return fail(403, { invalid: true, step: 'password' });
    const email = text(data, 'email', 320).toLowerCase();
    const password = raw(data, 'password', 1_024);
    if (!email || !password) return genericFailure();
    try {
      const result = await authenticatePassword(event, email, password);
      await saveWorkosSession(event, result);
    } catch (error) {
      if (await saveEmailVerificationChallenge(event, error)) {
        return { step: 'verify', notice: 'Enter the verification code sent to your email.' };
      }
      try {
        const mfa = await beginTotpChallenge(event, error);
        if (mfa?.enrollment) {
          return {
            step: 'mfa-enroll',
            notice: 'Add Piqae to your authenticator, then enter the current code.',
            enrollment: mfa.enrollment
          };
        }
        if (mfa) {
          return { step: 'mfa', notice: 'Enter the current code from your authenticator.' };
        }
      } catch {
        return genericFailure();
      }
      if (isAdvancedChallenge(error)) {
        redirect(303, `/auth/login?hosted=1&return_to=${encodeURIComponent(target(data))}`);
      }
      return genericFailure();
    }
    redirect(303, target(data));
  },

  signup: async (event) => {
    if (authMode !== 'workos') return genericFailure('signup');
    const data = await form(event, 'signup');
    if (!data) return fail(403, { invalid: true, step: 'signup' });
    const email = text(data, 'email', 320).toLowerCase();
    const password = raw(data, 'password', 1_024);
    if (!email || password.length < 12) return genericFailure('signup');
    try {
      const result = await registerPassword(event, email, password);
      await saveWorkosSession(event, result);
    } catch (error) {
      if (await saveEmailVerificationChallenge(event, error)) {
        return { step: 'verify', notice: 'Enter the verification code sent to your email.' };
      }
      return genericFailure('signup');
    }
    redirect(303, target(data));
  },

  magicStart: async (event) => {
    if (authMode !== 'workos') return genericFailure('magic');
    const data = await form(event, 'magic-start');
    if (!data) return fail(403, { invalid: true, step: 'magic' });
    const email = text(data, 'email', 320).toLowerCase();
    if (!email) return genericFailure('magic');
    try {
      await beginMagicAuth(event, email);
    } catch {
      // Deliberately return the exact success representation. The subsequent
      // code submission has no sealed server state and fails generically.
    }
    return { step: 'magic-code', notice: 'If that address can sign in, a code is on its way.' };
  },

  magicComplete: async (event) => {
    if (authMode !== 'workos') return genericFailure('magic-code');
    const data = await form(event, 'magic-complete');
    if (!data) return fail(403, { invalid: true, step: 'magic-code' });
    const code = text(data, 'code', 12);
    try {
      const result = await completeMagicAuth(event, code);
      await saveWorkosSession(event, result);
    } catch {
      return genericFailure('magic-code');
    }
    redirect(303, target(data));
  },

  verify: async (event) => {
    if (authMode !== 'workos') return genericFailure('verify');
    const data = await form(event, 'email-verification');
    if (!data) return fail(403, { invalid: true, step: 'verify' });
    try {
      const result = await completeEmailVerification(event, text(data, 'code', 12));
      await saveWorkosSession(event, result);
    } catch {
      return genericFailure('verify');
    }
    redirect(303, target(data));
  },

  mfa: async (event) => {
    if (authMode !== 'workos') return genericFailure('mfa');
    const data = await form(event, 'mfa');
    if (!data) return fail(403, { invalid: true, step: 'mfa' });
    try {
      const result = await completeTotp(event, text(data, 'code', 12));
      await saveWorkosSession(event, result);
    } catch {
      return genericFailure('mfa');
    }
    redirect(303, target(data));
  },

  resetRequest: async (event) => {
    if (authMode !== 'workos') return genericFailure('forgot');
    const data = await form(event, 'reset-request');
    if (!data) return fail(403, { invalid: true, step: 'forgot' });
    const email = text(data, 'email', 320).toLowerCase();
    try {
      if (email) await requestPasswordReset(email);
    } catch {
      // Deliberately indistinguishable to prevent account enumeration.
    }
    return { step: 'forgot', notice: 'If that address has an account, reset instructions are on their way.' };
  },

  reset: async (event) => {
    if (authMode !== 'workos') return genericFailure('reset');
    const data = await form(event, 'reset-complete');
    if (!data) return fail(403, { invalid: true, step: 'reset' });
    const token = text(data, 'token', 2_048);
    const password = raw(data, 'password', 1_024);
    if (!token || password.length < 12) return genericFailure('reset');
    try {
      await resetPassword(token, password);
    } catch {
      return genericFailure('reset');
    }
    return { step: 'password', notice: 'Password updated. Sign in with your new password.' };
  }
};

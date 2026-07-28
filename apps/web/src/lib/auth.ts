export type AuthMode = 'hosted' | 'oidc' | 'local';

export interface Viewer {
  id: string;
  email: string;
  name: string | null;
  workspaceId: string;
  roles: string[];
}

export interface AuthBoundary {
  mode: AuthMode;
  viewer(): Promise<Viewer | null>;
  signInUrl(returnTo?: string): string;
  signOutUrl(returnTo?: string): string;
  accessToken(): Promise<string | undefined>;
}

/**
 * Browser-safe auth boundary. Hosted WorkOS, generic OIDC, and local-owner
 * sessions terminate at same-origin server routes. Provider secrets and the
 * short-lived API token are never exposed to Svelte components.
 */
export function createAuthBoundary(mode: AuthMode = 'hosted'): AuthBoundary {
  const parameter = (returnTo?: string) =>
    returnTo ? `?return_to=${encodeURIComponent(returnTo)}` : '';
  return {
    mode,
    viewer: async () => {
      const response = await fetch('/auth/session', { headers: { accept: 'application/json' } });
      if (response.status === 401) return null;
      if (!response.ok) throw new Error('Unable to load the current Spool session');
      return (await response.json()) as Viewer;
    },
    signInUrl: (returnTo) => `/auth/login${parameter(returnTo)}`,
    signOutUrl: (returnTo) => `/auth/logout${parameter(returnTo)}`,
    accessToken: async () => {
      const response = await fetch('/auth/token', { method: 'POST' });
      if (!response.ok) return undefined;
      const body = (await response.json()) as { access_token: string };
      return body.access_token;
    }
  };
}

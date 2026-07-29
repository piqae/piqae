declare global {
  namespace App {
    interface Locals {
      auth?: import('@workos/authkit-sveltekit').AuthKitAuth;
      authMode: 'workos' | 'local' | 'demo';
      localSessionToken?: string;
    }

    interface PageData {
      mode?: 'mock' | 'live';
      dashboardMode?: 'live' | 'demo';
      viewer?: {
        id: string;
        email: string;
        name: string | null;
        organizationId: string | null;
        role: string | null;
      } | null;
    }
  }
}

export {};

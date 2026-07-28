declare global {
  namespace App {
    interface Locals {
      auth?: import('@workos/authkit-sveltekit').AuthKitAuth;
      authMode: 'workos' | 'local' | 'demo';
    }

    interface PageData {
      mode?: 'mock' | 'live';
      dashboardMode?: 'live' | 'demo';
      viewer?: {
        id: string;
        email: string;
        name: string | null;
        organizationId: string | null;
      } | null;
    }
  }
}

export {};

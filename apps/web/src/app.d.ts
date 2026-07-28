declare global {
  namespace App {
    interface Locals {
      auth?: import('@workos/authkit-sveltekit').AuthKitAuth;
      authMode: 'workos' | 'local' | 'demo';
    }

    interface PageData {
      mode?: 'mock' | 'live';
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

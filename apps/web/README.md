# Spool web

SvelteKit dashboard and documentation application. The visual system uses
original components and warm OKLCH tokens inspired by the calm density of
modern developer tools. It has no runtime font, analytics, or image dependency.

## Development

```sh
pnpm --filter @spool/web dev
pnpm --filter @spool/web check
pnpm --filter @spool/web test
```

The checked-in demo view model is deterministic and is used only when running
the UI in mock mode. `src/lib/api.ts` contains the separate live adapter for
the canonical `contracts/openapi/spool-v1.yaml` API.

## Deployment targets

Vercel is the hosted default:

```sh
pnpm --filter @spool/web build:vercel
```

The adapter is pinned to the Vercel Node.js 22 runtime so builds are
deterministic even when contributors use newer Node versions.

Self-hosting produces a normal Node server:

```sh
pnpm --filter @spool/web build:self-host
PORT=3000 node apps/web/build-node
```

Set `SPOOL_DEPLOYMENT_TARGET=node` in generic container builds. Authentication
providers terminate behind same-origin `/auth/*` routes; WorkOS secrets, OIDC
secrets, local-owner sessions, and short-lived API tokens never enter Svelte
components.

## Authentication modes

Copy `.env.example` and choose one explicit mode:

- `workos` configures the official `@workos/authkit-sveltekit` integration,
  sealed sessions, callback, sign-in/out endpoints, and protected dashboard.
- `local` does not initialize WorkOS. The self-hosted control plane or a
  trusted reverse proxy owns the user session.
- `demo` exposes a deterministic local viewer and must never be public.

Set the WorkOS callback to `/auth/callback` and sign-in endpoint to
`/auth/login`. All four `WORKOS_*` values are mandatory in hosted mode. The
package is pinned to verified `@workos/authkit-sveltekit@0.3.0`; the similarly
named `@workos-inc/authkit-sveltekit` does not exist in npm.

## Browser checks

```sh
pnpm --filter @spool/web test:e2e
```

Playwright covers desktop Chromium and a narrow mobile viewport, semantic
navigation, job truth labels, responsive overflow, documentation, and the
hosted authentication redirect boundary.

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

All dashboard pages load through SvelteKit server routes. Live mode is the
default and never falls back to demo data after an error. To opt into clearly
labelled local data, set `PUBLIC_SPOOL_DASHBOARD_MODE=demo`. Live mode reads
`PUBLIC_SPOOL_API_URL`. Hosted WorkOS sessions forward their verified OIDC
access token directly from sealed server locals to the control plane; the
token is never returned from an application endpoint, included in page data,
or placed in browser bundles. `SPOOL_DASHBOARD_API_KEY` is an explicit
server-only fallback for local/self-host deployments without user OIDC.
Live browser updates use a same-origin `/api/events` SSE proxy. That proxy
adds authentication server-side, forwards `Last-Event-ID`, disables buffering,
and streams bytes without placing credentials in the URL. The dashboard
throttles event-driven data invalidation to avoid request storms.

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
  sealed sessions, callback, sign-in/out endpoints, protected dashboard, and
  server-side bearer forwarding to the control plane's OIDC verifier.
- `local` does not initialize WorkOS. The self-hosted control plane or a
  trusted reverse proxy owns the user session. Set a scoped
  `SPOOL_DASHBOARD_API_KEY` only when the dashboard requires a service-key
  fallback.
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

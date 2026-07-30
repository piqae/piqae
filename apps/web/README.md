# Piqae web

SvelteKit dashboard and documentation application. The dashboard uses the
native Svelte [Piqae UI](src/lib/components/ui/README.md) system: neutral OKLCH
tokens, compact controls, restrained blue brand/focus colour, and semantic
operational states. It has no runtime font or image dependency.

## Development

```sh
pnpm --filter @piqae/web dev
pnpm --filter @piqae/web check
pnpm --filter @piqae/web test
```

The checked-in demo view model is deterministic and is used only when running
the UI in mock mode. `src/lib/api.ts` contains the separate live adapter for
the canonical `contracts/openapi/piqae-v1.yaml` API.

All dashboard pages load through SvelteKit server routes. Live mode is the
default and never falls back to demo data after an error. To opt into clearly
labelled local data, set `PUBLIC_PIQAE_DASHBOARD_MODE=demo`. Live mode reads
`PUBLIC_PIQAE_API_URL`. Hosted WorkOS sessions forward their verified OIDC
access token directly from sealed server locals to the control plane; the
token is never returned from an application endpoint, included in page data,
or placed in browser bundles. `PIQAE_DASHBOARD_API_KEY` is an explicit
server-only fallback for local/self-host deployments without user OIDC.
Live browser updates use a same-origin `/api/events` SSE proxy. That proxy
adds authentication server-side, forwards `Last-Event-ID`, disables buffering,
and streams bytes without placing credentials in the URL. The dashboard
throttles event-driven data invalidation to avoid request storms.

### Release downloads

`/downloads` is rendered from a server-only, schema-versioned artifact
manifest. Set `PIQAE_RELEASE_MANIFEST_JSON` to publish exact artifact URLs,
versions, platform and architecture targets, minimum OS versions, SHA-256
values, signing state, release notes, and older releases. The parser accepts
only HTTPS release URLs and refuses a `supported` claim unless the artifact has
a direct download, SHA-256, and verified platform-signing state.

When the setting is absent, the checked-in manifest reflects the repository
support matrix: Windows is Development-only and macOS/Linux are Preview, with
no fabricated download link or checksum. Browser user-agent hints highlight a
likely platform but never change the server-owned release status. The Add Node
journey leads with short-lived browser pairing; manual enrolment tokens remain
an explicit fallback in the node dialog.

Railway hosts the private release origin in the dedicated `piqae-releases`
bucket. Configure the web service with the `PIQAE_RELEASES_S3_*` values from
`.env.example`; the runtime performs reads only. Keep these credentials
distinct from both release publishing credentials and the `piqae-documents`
print-object credentials, and enforce read-only scope when the provider
supports it.
The constrained route maps:

```text
/releases/stable/<artifact> -> native/stable/<artifact>
/releases/preview/<artifact> -> native/preview/<artifact>
```

and returns a short-lived signed redirect. Missing configuration, an invalid
filename, or an absent object returns not found. The stable update feed URLs
are:

```text
https://downloads.piqae.com/releases/stable/appcast-macos.xml
https://downloads.piqae.com/releases/stable/appcast-windows.xml
```

Those paths being routable does not mean a release exists. Do not set an
artifact to `supported` or publish a stable appcast until native signature,
checksum, SBOM, provenance, installation, upgrade, and rollback evidence has
passed and [`release/support-matrix.yaml`](../../release/support-matrix.yaml)
agrees. See
[`docs/operations/native-release-publishing.md`](../../docs/operations/native-release-publishing.md).

### Local Mac node

The Local node page can manage a Piqae node running on the same machine as
the web server. Enable its same-origin proxy with both server-only values:

```sh
PIQAE_LOCAL_AGENT_URL=http://127.0.0.1:17890
PIQAE_LOCAL_AGENT_TOKEN_FILE=/absolute/path/to/local.token
```

The URL is restricted to loopback addresses. SvelteKit reads the token file
for each upstream request and sends the credential only to the local agent;
the path, token, and authorization header never enter browser data or browser
storage. When either setting is absent, local access remains disabled with an
explicit setup message.

## Deployment target

The hosted and self-hosted dashboard use the same normal Node server:

```sh
pnpm --filter @piqae/web build:self-host
PORT=3000 node apps/web/build-node
```

Piqae Cloud runs this image on Railway through
[`deploy/docker/Dockerfile.web`](../../deploy/docker/Dockerfile.web). The
production and staging services select `/railway.web.toml`; both use `/healthz`
as their deployment gate. Authentication providers terminate behind
same-origin `/auth/*` routes; WorkOS secrets, OIDC secrets, local-owner
sessions, and short-lived API tokens never enter Svelte components.

## Authentication modes

Copy `.env.example` and choose one explicit mode:

- `workos` configures the official `@workos/authkit-sveltekit` integration,
  sealed sessions, callback, sign-in/out endpoints, protected dashboard, and
  server-side bearer forwarding to the control plane's OIDC verifier.
- `local` does not initialize WorkOS. The self-hosted control plane or a
  trusted reverse proxy owns the user session. Set a scoped
  `PIQAE_DASHBOARD_API_KEY` only when the dashboard requires a service-key
  fallback.
- `demo` exposes a deterministic local viewer and must never be public.

Set the WorkOS callback to `/auth/callback` and sign-in endpoint to
`/auth/login`. All four `WORKOS_*` values are mandatory in hosted mode. The
package is pinned to verified `@workos/authkit-sveltekit@0.3.0`; the similarly
named `@workos-inc/authkit-sveltekit` does not exist in npm.

## Browser checks

```sh
pnpm --filter @piqae/web test:e2e
```

Playwright covers desktop Chromium and a narrow mobile viewport, semantic
navigation, job truth labels, responsive overflow, documentation, and the
hosted authentication redirect boundary.

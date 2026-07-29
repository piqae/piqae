# Piqae domain topology

Piqae uses stable public contracts in front of replaceable compute origins.
Marketing, dashboard, pairing, API and native node traffic do not require
separate Railway services.

## Canonical public hosts

| Host | Purpose | Initial origin |
|---|---|---|
| `piqae.com` | Marketing, pricing, security and product pages | SvelteKit web |
| `www.piqae.com` | Permanent redirect to `piqae.com` | Cloudflare edge |
| `app.piqae.com` | Operator dashboard and hosted human sessions | SvelteKit web |
| `connect.piqae.com` | Node pairing and platform customer onboarding | Redirect to the canonical app pairing flow |
| `docs.piqae.com` | Developer-documentation entry point | Existing web documentation routes |
| `downloads.piqae.com` | Signed native installer entry point | Existing web downloads route |
| `api.piqae.com` | Stable SDK, REST and webhook-management API | Rust control plane |
| `sync.piqae.com` | Native node sync, lease and status traffic | The same Rust control plane initially |

`deploy/cloudflare/domain-router` owns these hostnames as Cloudflare Worker
Custom Domains. Cloudflare creates their DNS records and certificates. The
router has only two origins, `WEB_ORIGIN` and `API_ORIGIN`; either origin can
move without changing an SDK base URL or an installed node.

The edge never makes tenant-authorisation decisions. API keys, device
signatures, workspace bindings and environment bindings remain authoritative
in the control plane.

## Reserved hosts

Do not point these names at a generic application until their contracts exist:

- `auth.piqae.com`: WorkOS AuthKit production custom domain;
- `uploads.piqae.com`: length- and digest-bound direct document ingress;
- `updates.piqae.com`: signed update metadata and immutable artifacts;
- `status.piqae.com`: status page on a provider independent of the application
  data plane;
- `customers.piqae.com`: Cloudflare for SaaS fallback origin for verified
  enterprise vanity domains;
- `support.piqae.com`: support and knowledge-base entry point.

Installer binaries may be linked from `downloads.piqae.com`, but native update
trust must not use that marketing host. Update metadata needs its own signing
and rollback protections before `updates.piqae.com` becomes active.

## Authentication boundaries

The hosted product uses two human surfaces:

1. `app.piqae.com` for Piqae workspace operators;
2. `connect.piqae.com` for node pairing and platform-customer onboarding.

V1 may redirect Connect into a pairing route on the App host. Before branded
customer portals launch, create separate WorkOS applications for **Piqae
Admin** and **Piqae Connect**. They may share users and organisations while
retaining separate redirect URIs and session policy.

The tray never stores a human WorkOS session. Browser approval binds a locally
generated device public key to an exact account, workspace and Test or Live
environment. The node exchanges and subsequently syncs against canonical
Piqae API hosts, not a customer vanity domain.

## Enterprise and platform custom domains

Enterprise platform customers may attach a subdomain they control, such as
`print.customer.example`, to a branded Connect portal. V1 custom domains do not
replace:

- `api.piqae.com`;
- `sync.piqae.com`;
- `updates.piqae.com`;
- the Piqae operator dashboard.

That boundary keeps print delivery, offline recovery, updater trust and API
credentials working when customer DNS is misconfigured.

A custom domain belongs to one platform account and one branding configuration,
not to a mutable display name. Hostname lookup selects a portal configuration
only; it never grants workspace access. A one-use invitation or device
authorisation still selects the exact managed account and environment.

### Lifecycle

1. Normalise the requested hostname with IDNA and reject IP addresses,
   localhost names, public suffixes and reserved Piqae names.
2. Create a globally unique pending claim scoped to the platform account.
3. Require customer TXT ownership validation.
4. Provision the hostname and TLS through a `CustomDomainProvider` abstraction,
   initially backed by Cloudflare for SaaS.
5. Require ownership, hostname routing, TLS and actual DNS target checks before
   marking it active.
6. Monitor DNS and certificate state continuously. A degraded vanity domain
   never revokes nodes or stops queued jobs.
7. On removal, block new portal sessions, remove the edge route, retain audit
   evidence, and hold the hostname for a cooldown before allowing it to be
   claimed again.

Every customer receives a canonical fallback URL under
`connect.piqae.com/p/<opaque-platform-slug>`. It remains usable during a vanity
domain incident.

### Central authentication exchange

Arbitrary customer domains are not registered as OAuth callbacks. WorkOS
returns to a canonical Piqae callback. Piqae then creates a 30–60 second,
single-use, hashed exchange code bound to:

- the authenticated user;
- exact verified custom domain;
- platform account and managed customer account;
- requested return path and CSRF state.

The vanity-domain backend exchanges that code server-to-server and sets a
host-only `__Host-` Secure, HttpOnly, SameSite cookie. Parent-domain cookies,
workspace IDs supplied by browsers and wildcard CORS are prohibited.

### Edge-to-origin security

The vanity-domain edge must:

- reject unknown hosts with `421`;
- overwrite untrusted tenant and forwarding headers;
- send a short-lived HMAC-signed origin assertion containing the domain claim
  ID, timestamp and request digest;
- rate-limit by platform, account, domain and source;
- use exact Origin and CSRF validation;
- generate absolute URLs from stored canonical origins, never raw forwarded
  headers.

The origin verifies the assertion and resolves the active domain claim from
PostgreSQL or a bounded cache. Hostname alone is never sufficient authority.

## Self-hosting

A self-hosted deployment defaults to one operator-owned HTTPS origin with
`/app`, `/api/v1` and `/connect`. Split hosts are optional. Compose expects an
operator-managed reverse proxy and TLS; Helm accepts an exact host list and can
use the cluster's ingress or Gateway API and certificate manager.

Self-hosted nodes use the configured control-plane URL and are not dependent on
Cloudflare, WorkOS or a Piqae Cloud account.

## Change and rollback

- Change Cloudflare origin variables only after the replacement origin passes
  readiness and browser checks.
- Keep the previous origin deployed throughout the observation window.
- Do not redirect API POST requests during an origin migration; proxy them.
- Node endpoint migration requires signed control-plane metadata and N/N-1
  protocol support.
- Removing a vanity domain never deletes profiles, jobs, device credentials or
  local queues.

The checked-in edge router is intentionally small enough to audit. Its route
tests are a release gate:

```console
node --test deploy/cloudflare/domain-router/router.test.mjs
pnpm dlx wrangler deploy --dry-run \
  --config deploy/cloudflare/domain-router/wrangler.jsonc
```

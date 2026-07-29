# Piqae edge domain router

This small Cloudflare Worker owns Piqae's canonical public hostnames and forwards
them to two replaceable origins:

- `WEB_ORIGIN`: the SvelteKit marketing, dashboard, pairing, documentation and
  download application;
- `API_ORIGIN`: the Rust control plane used by SDKs and native nodes.

Cloudflare Custom Domains create the required DNS records and certificates.
There is no Railway or Vercel service per hostname. `api.piqae.com` and
`sync.piqae.com` intentionally share the control-plane origin while retaining
separate stable contracts for future scaling.

Run the deterministic routing tests:

```console
node --test deploy/cloudflare/domain-router/router.test.mjs
```

Deploy after both origins pass their own readiness checks:

```console
pnpm dlx wrangler deploy \
  --config deploy/cloudflare/domain-router/wrangler.jsonc
```

Changing an origin does not change the public API, pairing URLs or installed
node configuration. Always keep `API_ORIGIN` and `WEB_ORIGIN` credential-free
HTTPS URLs. The router overwrites forwarding headers and rejects unknown hosts.

The enterprise vanity-domain product is separate. Customer domains resolve to a
verified account portal through Cloudflare for SaaS; they must never become API,
node-sync or updater origins.

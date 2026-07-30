# Piqae marketing CMS

This is a private Payload application for marketing copy, comparison evidence,
displayed prices, and cleared media. It must use a database or schema and role
that cannot modify the Piqae control-plane tables.

```console
cp .env.example .env
pnpm --filter @piqae/cms generate:types
pnpm --filter @piqae/cms dev
```

The first user becomes an administrator. Later anonymous user creation is
denied. Editors manage content, reviewers manage evidence, and administrators
manage users and destructive operations.

Published content triggers `MARKETING_DEPLOY_HOOK_URL`. The pricing collection
may override only the Free and Pro headline prose. Plan names, prices, limits,
overages, retention policy, features, and CTAs are owned by the SvelteKit server
catalog and are not CMS fields.

The authenticated hourly route at `/api/cron/pricing-drift` compares configured
Stripe prices with that server catalog through `PRICING_DRIFT_CHECK_URL` and
alerts `PRICING_DRIFT_WEBHOOK_URL`. CMS publication cannot change transactional
pricing.

When all R2 values are configured, Payload stores `media` objects through the S3
adapter. Cleared C4 assets require alt text, ownership, privacy review, and a
rights confirmation before publication.

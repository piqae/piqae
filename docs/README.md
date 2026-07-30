# Piqae developer documentation

Piqae Cloud is the default path: add the SDK to a trusted backend, create one
isolated account per customer, pair their nodes in a browser, and send durable
print jobs through the same API used by the dashboard.

## Start here

- [Hosted headless quickstart](api/platform-headless-quickstart.md) — a SaaS,
  marketplace, fulfilment, or design application serving many customers.
- [Single-workspace quickstart](api/quickstart.md) — one organisation adding
  printing to its own backend.
- [Cloud node setup](getting-started/cloud.md) — install, pair, discover a
  printer, capture a native profile, and send a test job.
- [PrintNode migration](api/printnode-migration.md) — switch the tested
  compatibility subset, then adopt native Piqae resources.

## Build an integration

- [Platform accounts](api/platform-service-accounts.md)
- [Authentication](api/authentication.md)
- [Uploads and design applications](api/uploads-and-design-apps.md)
- [Cloud billing and usage](api/billing-and-usage.md)
- [Jobs, offline nodes, and delivery truth](printing/jobs-and-statuses.md)
- [Idempotency](api/idempotency.md)
- [Signed webhooks](api/webhooks.md)
- [Printers and native profiles](printing/printers.md)
- [Complex vendor drivers](printing/complex-drivers.md)

Platform application code can create, retrieve, list, and archive customer
accounts, then manage their nodes, printers, profiles, targets, uploads, jobs,
API keys, webhooks, and usage through an account-scoped SDK client. Creating,
rotating, or revoking the first platform credential remains an operator or
hosted account-setup action.

## Project

- [Open source and self-hosting](open-source.md)
- [Contributing](contributing/README.md)
- [Operations](operations/README.md)
- [Architecture record](00-vision-and-scope.md)

## Status language

- **Implemented:** code exists in this repository.
- **Tested:** the named automated or physical test actually ran.
- **Preview:** usable for evaluation, but release gates remain.
- **Supported:** covered by a published stable support tier.
- **Disabled:** present for development only; not a production release claim.
- **Planned:** design only.

The authoritative release tiers are
[`release/support-matrix.yaml`](../release/support-matrix.yaml) and
[`release/native-bundle-status.md`](../release/native-bundle-status.md). A job
accepted by an operating-system spooler is not proof that ink reached paper.

Repository-relative links retain the literal `.md` extension so the same pages
work in GitHub, source archives, raw Markdown readers, and generated
`llms-full.txt`.

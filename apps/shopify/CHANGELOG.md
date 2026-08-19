# Shopify app changelog

All notable merchant-facing changes to the Piqae Shopify app are recorded here.
The hosted runtime, Shopify app version, and `shopify-v*` tag must identify the
same reviewed source commit.

## 0.2.0 - Unreleased

### Added

- Commit-bound hosted health evidence for protected Shopify promotions.
- Automated post-CI staging deployment and manual production release workflow.
- Single-store pilot, rollback, privacy, and ongoing operations runbook.

### Existing preview capabilities

- Embedded Shopify Admin application for order and draft-order documents.
- Admin, POS, and customer-account extensions.
- Semantic document template editor and authenticated PDF fallback.
- Durable PostgreSQL sessions, installations, workflows, and webhook receipts.
- Direct Piqae job registration with explicit fake/local/live runtime policy.

### Known limitations

- Shopify approval and live-store/POS evidence remain open.
- Standard order-history scope applies; `read_all_orders` is not requested.
- International typography, independent security review, production soak, and
  physical-printer certification remain open release gates.

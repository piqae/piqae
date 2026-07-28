# Contributing to Spool

Spool accepts contributions under the license of the component being changed.
Every commit must include a Developer Certificate of Origin sign-off.

1. Open an issue for substantial behavior or protocol changes.
2. Add tests for observable behavior.
3. Run `cargo fmt --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, and `pnpm check`.
4. Keep protocol and database changes backward compatible within the documented support window.
5. Do not include customer documents, credentials, or production logs in issues or fixtures.

See `GOVERNANCE.md`, `SECURITY.md`, and `docs/execution/v1.md`.

# Development setup

**Status:** implemented source workflow.

Prerequisites are Rust 1.88, Node 22, pnpm, PostgreSQL for control-plane tests,
and platform print tooling. macOS shell development also needs Swift; Linux
CUPS builds need CUPS development headers.

```sh
cargo test --workspace --all-targets
corepack enable
pnpm install --frozen-lockfile
pnpm --filter @piqae/web test
pnpm --filter @piqae/web check
```

Run the local agent and dashboard in separate terminals. Use disposable data
directories and test printers; never point development jobs at production
label stock without an operator at the device.

Detailed commands and repository boundaries are in
[`contributing/development.md`](../contributing/development.md),
[`contributing/testing.md`](../contributing/testing.md), and
[`03-architecture-and-stack.md`](../03-architecture-and-stack.md).

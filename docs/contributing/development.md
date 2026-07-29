# Development

## Toolchain

Spool uses Rust 1.88, Node.js 22, pnpm 11.4, Swift on macOS, and Docker for the
self-hosted stack. `mise.toml` is the canonical cross-language tool declaration.

```console
mise install
pnpm install --frozen-lockfile
cargo xtask doctor
```

Without mise, install the same versions and run the doctor directly.

## Common commands

```console
cargo xtask dev
cargo xtask dev web
cargo xtask dev agent
cargo xtask test changed
cargo xtask test all
cargo xtask fixture reset
```

`dev` defaults to the deterministic demo dashboard. `dev agent` starts the
local agent with the fake executor and stores disposable state in `.spool-dev`.
Run the two commands in separate terminals when working across the web and
agent boundary.

The local agent API binds to `127.0.0.1:39100`. Its token is generated under
`.spool-dev`; do not commit or paste it into logs.

## Repository shape

- `crates/` and `bins/`: Rust agent, protocol, executors, server, and CLI
- `apps/web/`: SvelteKit dashboard
- `sdk/`: public SDKs
- `shells/`: thin platform-native menu/tray shells
- `contracts/`: externally visible API contracts
- `migrations/`: append-only database evolution
- `xtask/`: safe contributor automation

Use installed OS printer drivers. Do not add a Chromium desktop runtime or move
durable job state into a tray application.

## Safe local operation

No contributor command prints to physical hardware by default. Do not set
`SPOOL_ALLOW_PHYSICAL_TESTS=1` unless a human has named the printer, stock, and
expected output for that run. Never make physical tests part of ordinary CI.

# Piqae

Piqae is open-source, low-resource local and remote printing infrastructure. It
uses installed operating-system printers and drivers, keeps a durable queue at
both the service and device edges, and never equates a spooler handoff with
proof that paper physically printed.

The repository now contains:

- a headless agent for Windows, macOS, Linux, and low-power Linux devices;
- a local-first mode that needs no control-plane server;
- an optional self-hosted control plane for remote and offline printing;
- an optional multi-tenant SaaS built from the same open-source control plane;
- a compatibility API that allows existing legacy-provider integrations to change
  their base URL and credentials instead of being rewritten;
- durable, observable delivery through the control-plane queue, agent queue,
  and operating-system spooler.

Working V1 source implementations of the shared agent, queues, control plane,
compatibility API, SDK, dashboard, documentation app, native executor
processes, and tray/menu shells live in this monorepo. Platform support remains
evidence-gated: source existing is not the same as a signed, physically
verified release.

## Quick start

Requirements are Rust 1.88, Node.js 22 or newer, pnpm 11.4, and Docker for the
self-hosted stack.

```sh
cargo test --workspace --locked
pnpm install --frozen-lockfile
pnpm check
pnpm test
```

Run the dashboard with visibly labelled deterministic data:

```sh
PIQAE_AUTH_MODE=demo PUBLIC_PIQAE_DASHBOARD_MODE=demo \
  pnpm --filter @piqae/web dev
```

Run the local agent against the disposable test executor:

```sh
cargo build -p piqae-agent -p piqae-fake-executor
cargo run -p piqae-agent -- \
  --mode local \
  --data-dir .piqae-dev \
  --executor process \
  --executor-path target/debug/piqae-fake-executor
```

The operational API binds only to loopback. Its randomly generated bearer token
is stored as mode `0600` in `.piqae-dev/local.token`. Use
`GET /v1/local/printers`, `POST /v1/jobs`, and `GET /v1/local/status` to exercise
the local durable print path.

For the API-only self-hosted stack:

```sh
cp deploy/self-host/.env.example deploy/self-host/.env
docker compose --env-file deploy/self-host/.env \
  -f deploy/self-host/docker-compose.yml up -d
```

See [self-hosting](docs/operations/self-hosting.md), the
[legacy-provider migration guide](docs/api/legacy-compatibility.md), and the
[OpenAPI contract](contracts/openapi/piqae-v1.yaml) before connecting real
printers. Production trace export is covered in the
[observability guide](docs/operations/observability.md).

## Packages

- TypeScript: `npm install @piqae/sdk`
- Containers: `ghcr.io/piqae/piqae/server`, `ghcr.io/piqae/piqae/migrate`, and
  `ghcr.io/piqae/piqae/web`
- Native candidates: draft releases in
  [GitHub Releases](https://github.com/piqae/piqae/releases)

Publishing remains fail-closed until the repository has moved to
`piqae/piqae` and the npm trusted publisher has been configured. See the
[package publishing guide](docs/operations/package-publishing.md).

## Documentation

1. [Vision, principles, and scope](docs/00-vision-and-scope.md)
2. [the legacy service capability inventory](docs/01-legacy-compatibility-inventory.md)
3. [Product requirements and user experience](docs/02-product-requirements.md)
4. [Proposed architecture and technology choices](docs/03-architecture-and-stack.md)
5. [Queues, protocol, and job state machines](docs/04-protocol-queues-and-state.md)
6. [Platform printing and native integration](docs/05-platform-printing.md)
7. [Drop-in API compatibility](docs/06-api-compatibility.md)
8. [Security, privacy, and observability](docs/07-security-observability-and-operations.md)
9. [Testing and reliability strategy](docs/08-testing-and-reliability.md)
10. [Delivery roadmap and resourcing](docs/09-roadmap.md)
11. [Decisions, risks, and open questions](docs/10-decisions-risks-and-open-questions.md)
12. [Native API and data model](docs/11-native-api-and-data-model.md)
13. [Open-source, SaaS, and build strategy](docs/12-open-source-saas-and-build-plan.md)
14. [Superseded Go-first evaluation](docs/13-two-day-mvp-and-stack-decision.md)
15. [Long-term native architecture and 48-hour production plan](docs/14-long-term-native-architecture-and-48-hour-production.md)
16. [Linear-aligned visual system](docs/15-linear-aligned-visual-system.md)
17. [Native print profiles, stock, and routing](docs/16-native-print-profiles-stock-and-routing.md)
18. [Research sources](docs/references.md)

## Architecture

Piqae uses one Rust agent core with small platform-specific executor processes,
not three unrelated desktop applications. The service owns identity, SQLite,
queueing, networking, printing, and recovery. Tray/menu applications are thin
native shells and never own job state. Piqae does not ship Electron or a bundled
Chromium runtime.

The Rust control plane uses PostgreSQL leases, durable per-agent commands,
event/outbox tables, and S3-compatible object storage. Agents use signed HTTPS
polling; there is no required broker or permanent socket gateway. The SvelteKit
dashboard runs as a standard Node container on Railway or self-hosted and uses the official
WorkOS AuthKit integration in hosted mode. The canonical TypeScript SDK and
`piqaectl` cover the native API.

The hosted and loopback web interfaces use an intentionally close Linear-like
visual language: calm warm-neutral surfaces, dense alignment, restrained
contrast, compact controls, fine typography, subtle borders, and fast motion.
This is visual-style alignment, not a copy of Linear's product layout, UX,
brand, icons, wording, or assets. The native tray/menu shells continue to follow
their operating-system conventions.

The first release target is the subset that replaces this organisation's
The legacy service usage. The compatibility surface is not represented as universal
The legacy service parity: scales, integrator subaccounts, billing, and additional
historical response quirks remain later layers and do not weaken the reliable
print path.

## Release truth

The checked-in [support matrix](release/support-matrix.yaml) is authoritative.
Windows, CUPS, and all signed installers remain Disabled or Preview until their
physical-printer, service restart, driver-option, code-signing, and update
gates pass. Windows PDF currently requires a separately installed and explicitly
configured SumatraPDF helper; see the
[backend notes](docs/architecture/windows-pdf-helper.md).

## Important semantic limit

No print relay can prove that ink reached paper on every printer. Many drivers
only report that a job was accepted by the operating-system spooler or sent to
the device. The API must distinguish `accepted_by_spooler`,
`printing`, `completed_reported`, and `delivery_uncertain`; it must never turn
“the spooler accepted this” into a false claim that the physical page printed.

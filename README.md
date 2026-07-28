# Open print relay

This repository is the planning workspace for an open-source, low-resource,
self-hostable alternative to PrintNode.

The intended product is:

- a headless agent for Windows, macOS, Linux, and low-power Linux devices;
- a local-first mode that needs no control-plane server;
- an optional self-hosted control plane for remote and offline printing;
- an optional multi-tenant SaaS built from the same open-source control plane;
- a compatibility API that allows existing PrintNode integrations to change
  their base URL and credentials instead of being rewritten;
- durable, observable delivery through the control-plane queue, agent queue,
  and operating-system spooler.

The project is currently in the design and validation stage. No implementation
exists yet.

## Documentation

1. [Vision, principles, and scope](docs/00-vision-and-scope.md)
2. [PrintNode capability inventory](docs/01-printnode-capability-inventory.md)
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
17. [Research sources](docs/references.md)

## Current recommendation

Build one Rust agent core with small platform-specific printing adapters, not
three unrelated desktop applications. Compile it into clean native artifacts
for Windows, macOS, and Linux. The service owns identity, SQLite, queueing,
networking, printing, and recovery. V0.1 tray/menu applications are thin native
platform shells and never own job state. Include them for Windows, macOS, and
supported Linux desktops. Do not ship Electron or a bundled Chromium runtime.

Use SvelteKit and TypeScript for the first control plane and dashboard, deployed
to Vercel with WorkOS AuthKit, Neon PostgreSQL, and private object storage.
Agents initially long-poll or poll a durable PostgreSQL job queue over HTTPS.
Do not require a broker or permanent WebSocket gateway for the first vertical
slice. A dedicated Go connection service can be introduced later if measured
latency or fleet scale requires it.

The hosted and loopback web interfaces use an intentionally close Linear-like
visual language: calm warm-neutral surfaces, dense alignment, restrained
contrast, compact controls, fine typography, subtle borders, and fast motion.
This is visual-style alignment, not a copy of Linear's product layout, UX,
brand, icons, wording, or assets. The native tray/menu shells continue to follow
their operating-system conventions.

The target is a narrow but production-operated v0.1 in 48 hours, built through
parallel workstreams with one contract/state-machine owner and hard integration,
security, recovery, and physical-printer release gates. The detailed decision is
in [the 48-hour production plan](docs/14-long-term-native-architecture-and-48-hour-production.md).

The first production goal should be the subset that replaces this
organisation's PrintNode usage. Public PrintNode API compatibility can then be
expanded systematically. Scale support, integrator subaccounts, delegated
authentication, and billing are later parity layers and must not delay the
reliable print path.

## Important semantic limit

No print relay can prove that ink reached paper on every printer. Many drivers
only report that a job was accepted by the operating-system spooler or sent to
the device. The API must distinguish `accepted_by_spooler`,
`printing`, `completed_reported`, and `delivery_uncertain`; it must never turn
“the spooler accepted this” into a false claim that the physical page printed.

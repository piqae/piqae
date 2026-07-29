# RFC process

Use an RFC for a new durable boundary, public protocol, security model,
platform support claim, native profile format, dependency with broad lock-in,
or behavior that can duplicate/lose physical output.

An RFC contains:

- problem, users, constraints, and explicit non-goals;
- current evidence and status language;
- proposed data/protocol/state changes;
- crash, retry, idempotency, security, privacy, and migration analysis;
- alternatives and why they were not selected;
- rollout, rollback, observability, testing, and release gates;
- unresolved decisions with named owners.

Open the RFC as Markdown under `docs/rfcs/NNNN-short-name.md` in a pull request.
Link relevant issues and existing numbered documents. Discussion changes the
RFC text; acceptance is recorded before broad implementation. Follow with an
ADR when the decision changes the long-term architecture.

An RFC approval is not a release certification. Update the support matrix only
after implementation and evidence gates pass. Emergency fixes may precede an
RFC when needed to stop data loss or unsafe printing, but the decision and
follow-up tests must be documented immediately afterward.

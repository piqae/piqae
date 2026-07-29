# Node updates

**Status:** signed update metadata handling exists in the protocol; automated
signed native updating is not a Supported release feature.

Current source bundles require an operator-managed upgrade:

1. Drain or pause new work.
2. Record active and delivery-uncertain jobs.
3. Back up the agent data directory and configuration.
4. Verify the new archive checksum from a trusted channel.
5. Stop the agent, replace binaries, and preserve identity/state.
6. Start the agent and verify health, printers, profiles, and queue recovery.
7. Submit one controlled profile test.
8. Keep the prior binaries until the observation window passes.

Checksums detect transfer corruption only; they do not replace code signing.
Windows and macOS artifacts are currently unsigned. Never run an update
command received through a print job, webhook, support bundle, or log.

Server protocol N and N-1 compatibility is the target policy. Upgrade servers
before a broad node rollout. See [platform upgrades](../operations/upgrades.md)
and [`contributing/releases.md`](../contributing/releases.md).

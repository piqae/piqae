---
name: test-piqae-web
description: Validate Piqae SvelteKit dashboard changes, including capability-aware cloud/self-host rendering, accessibility, and browser flows. Use for changes under apps/web or web-facing API contracts.
---

# Test Piqae web

1. Read `AGENTS.md` and `apps/web/AGENTS.md`.
2. Set `PIQAE_STATE_DIR` to an ignored worktree-local `.piqae-test-fixtures/web`.
3. Run `pnpm --filter @piqae/web check` and `pnpm --filter @piqae/web test`.
4. Start only virtual services through `cargo xtask dev`; never pass
   `--real-printers`.
5. Verify changed journeys in a browser at narrow desktop and tablet widths,
   including keyboard focus and cloud/self-host capability variants.
6. Capture the route, viewport, capability response, and test output as
   acceptance evidence.

Never log WorkOS cookies, refresh tokens, API keys, device codes, webhook
secrets, or document URLs. Stop services started by the skill and remove only
`.piqae-test-fixtures/web`; do not touch OS print queues.

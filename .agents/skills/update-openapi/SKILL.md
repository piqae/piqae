---
name: update-openapi
description: Update the Piqae native and PrintNode-compatible OpenAPI contract, regenerate SDK types, and prove additive compatibility. Use for any public route, request, response, error, or status change.
---

# Update OpenAPI

1. Read `AGENTS.md` and the compatibility rules.
2. Edit `contracts/openapi/piqae-v1.yaml` before implementation types.
3. Run the repository OpenAPI validation and TypeScript generation commands.
4. Run SDK tests and PrintNode migration contract tests.
5. Confirm V1 changes are additive and deprecated `/v1/agents` aliases remain
   compatible.
6. Record the operations changed, generated diff, and test outputs.

Use only redacted examples. Never embed working API keys, webhook secrets,
device codes, signed URLs, customer identifiers, or document content. Remove
only generator scratch files; keep deliberate generated outputs.

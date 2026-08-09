# Declarative document release audit

## Zero-copy artifact-to-job ownership

Migration 0034 adds append-only, tenant-fenced render-to-job references and a
database trigger that creates the ownership edge in the same transaction as
the job. Render cleanup excludes active references and job finalization marks
them released. The print endpoint uses a deterministic acquisition alias and
does not fetch, base64-encode, or rewrite artifact bytes. Empty-database
migration through schema version 35 was exercised against disposable
PostgreSQL 16; the existing cross-tenant document-reference probe passed.

This validates storage ownership and migration behavior, not physical output.
The fake-print suite remains the required executor boundary.

**Audit date:** 2026-08-09  
**Audited base commit:** `42f38e28c8105c1bce117037617399fa056c8108`  
**Candidate state:** uncommitted concept worktree; not a release candidate

This record covers non-physical evidence that can be produced on an arm64 macOS
host. It does not authorize publication and does not replace hosted CI,
independent security review, soak evidence, signing, notarisation, Windows
runtime validation, disaster-recovery rehearsal, or physical-printer
certification. `release/support-matrix.yaml` remains authoritative.

| Gate | Result | Exact evidence |
| --- | --- | --- |
| Compose interpolation | Pass | `docker compose --env-file deploy/self-host/.env.example -f deploy/self-host/docker-compose.yml config --quiet` |
| Helm schema and template | Pass | Helm 3.17.3 container: `helm lint deploy/helm/piqae` and `helm template piqae deploy/helm/piqae` |
| Terraform syntax/provider validation | Pass | Terraform 1.11.3 container: `fmt -check`, `init -backend=false`, and `validate`; Google 6.50.0 and Random 3.9.0 providers |
| Empty and N-1 PostgreSQL migration | Pass | `cargo test -p piqae-storage-postgres --test migrations --locked -- --nocapture`: 6 passed, including document creation, previous-schema upgrades, and legacy encryption migration |
| Tenant isolation and key retirement | Pass | `documents_migrate_and_enforce_tenant_scoped_references` and `document_key_retirement_waits_for_every_retained_ciphertext` passed against disposable PostgreSQL 16 |
| Mandatory PostgreSQL release gates | Pass | `release/tools/check_postgres_release_tests.py`: routing recovery, platform service accounts, platform service-account HTTP, and platform accounts passed |
| Rust format and strict Clippy | Pass | Reached and passed through `cargo xtask release check` |
| Rust workspace tests | Pass | Reached and passed through `cargo xtask release check`, including 77 control-plane and 11 renderer tests |
| JavaScript checks and tests | Pass with environment warning | Type checks passed; adapter 4, SDK 39, MCP 17, and web 139 tests passed. Host used Node 24.16.0 while the repository requests Node 22.x |
| JavaScript production builds | Pass with existing warnings | CMS, adapters, SDK, MCP, and web built. Web reported unresolved runtime licensed-font URLs and a chunk larger than 500 kB |
| Dependency policy | Pass | cargo-deny 0.20.2: advisories, bans, licences, and sources passed |
| macOS source validation | Pass | macOS 26.3 arm64: 54 Swift tests plus agent, local API, and CUPS executor suites passed |
| macOS unsigned package structure | Pass for local Preview only | Isolated `Piqae.app` 0.1.11 built; `PiqaeBuildChannel=unsigned-preview`, updates disabled, ad-hoc linker signature only |
| Windows executor cross-check | Pass | `cargo check -p piqae-executor-windows --target x86_64-pc-windows-gnu --locked` |
| Windows full agent/runtime/installer | Not run | ARM macOS host has no MinGW compiler, PowerShell, Windows runtime, DPAPI, Winspool, Inno Setup, or installer environment |
| Release check terminal gate | Expected fail | Every executable check passed; the final clean-worktree gate failed because the implementation is intentionally uncommitted |
| Signing, notarisation, provenance and publication | Not run | Requires reviewed tag and protected hosted release workflow; local artifacts are not release inputs |
| Physical printing | Not run | No printer or fixture was authorized; no physical print command ran |

## Open external evidence

- Run the Windows DPAPI, PDFium, tray, installer, update/rollback and uninstall
  suite on the supported x86-64 Windows hosted runner.
- Run the checked-in load/soak profile on production-equivalent compute and
  retain percentile, failure, queue-depth and resource evidence.
- Complete an independent cryptographic and untrusted-template security review.
- Rehearse database/object-store backup restoration and a Helm disruption and
  rollback in an isolated environment.
- Build signing, SBOM, checksum, provenance, notarisation and update evidence
  from a reviewed tag in the protected hosted workflow.
- Schedule physical PDF and raw-printer matrices separately with named devices,
  stock, profiles and safe fixtures.

Until those records exist, declarative document generation remains **Disabled**
for production in the support matrix even though the local implementation and
non-physical suites pass.

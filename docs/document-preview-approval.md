# Document preview approval

`POST /v1/business-document-renders/{render_id}/previews` retains the exact completed
render artifact behind a short-lived approval gate. It does not register a
print job. Preview TTL is caller-selected from 60 through 1,800 seconds and is
10 minutes by default.

The browser-facing application should proxy preview API calls. Piqae tenant
credentials and object-store locations must not enter a browser or extension.
The preview artifact endpoint authenticates the tenant and verifies its byte
length and SHA-256 digest before returning the PDF.

Approval claims a preview, then registers a job through the existing
idempotent, zero-copy document-artifact path. A retry with the same approval
key resumes safely after a process failure and returns the same job. Another
key or request is rejected. Cancellation is allowed only before approval;
closing a client without cancelling is handled by TTL expiry. Expired and
cancelled previews cannot be approved.

Migration `0038_business_document_cutover.sql` preserves the preview gate while
performing the explicit prerelease document-data reset. Preview/render/job
relationships repeat `workspace_id` and `environment_id` in their foreign keys.
Active preview gates prevent artifact cleanup, while cancellation or expiry
releases that retention without deleting the immutable render synchronously.

Validation evidence (2026-08-19): fresh and N-1 migrations ran against a
disposable PostgreSQL database; all eight migration suites passed, including
tenant-reference probes. The disposable container and database were removed.
Control-plane/storage tests, strict Clippy, OpenAPI regeneration checks, and 46
TypeScript SDK tests passed. These tests use no physical printer.

# Document preview approval

`POST /v1/printpacket/renders/{render_id}/previews` retains the exact completed
render artifact behind a short-lived approval gate. It does not register a
print job. Preview TTL is caller-selected from 60 through 1,800 seconds and is
10 minutes by default.

The browser-facing application should proxy preview API calls. Piqae tenant
credentials and object-store locations must not enter a browser or extension.
The preview artifact endpoint authenticates the tenant and verifies its byte
length and SHA-256 digest before returning the PDF.

Completed render responses can include a bounded `warnings` array of stable,
machine-readable codes. `document_data_missing` means an unguarded template
path was unavailable and its value was rendered blank; the preview and print
flow remain available so a user can review the result. Templates that use
`coalesce` or `exists` to handle absent data do not emit that warning. Invalid
document structure, unsupported renderer capabilities, unsafe resources, and
other failures that cannot produce a trustworthy PDF remain terminal.

Approval claims a preview, then registers a job through the existing
idempotent, zero-copy document-artifact path. A retry with the same approval
key resumes safely after a process failure and returns the same job. Another
key or request is rejected. Cancellation is allowed only before approval;
closing a client without cancelling is handled by TTL expiry. Expired and
cancelled previews cannot be approved.

The initial PrintPacket schema defines the preview gate directly. Preview/render/job
relationships repeat `workspace_id` and `environment_id` in their foreign keys.
Active preview gates prevent artifact cleanup, while cancellation or expiry
releases that retention without deleting the immutable render synchronously.

Release evidence is recorded by the current migration, control-plane, SDK, and
virtual-print test gates. These tests do not use a physical printer.

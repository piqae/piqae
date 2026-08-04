# Pairing and enrolment

**Status:** browser pairing, one-time connected enrolment, and in-place device
key rotation are implemented; installer integration and platform release tiers
vary.

For an interactive desktop install, start browser pairing:

```sh
piqae-agent \
  --data-dir /secure/piqae \
  --control-plane-url https://api.piqae.example.com \
  --pair \
  --enrolment-name 'Packing room'
```

The node creates its Ed25519 key locally and opens a short-lived browser
approval page. Confirm the node name, hostname, platform, and architecture,
then enter the eight-character code shown by the node. The device code expires
after ten minutes and is sent in a request body, never in a URL, so it is not
recorded by proxy or CDN access logs. Repeating the exchange with the same
code returns the same node identity, so a node interrupted mid-pairing can
retry without stranding a half-created node. The human web session is not
retained by the node.

Headless automation may use an administrator-created enrolment token through
the `PIQAE_ENROLMENT_TOKEN` environment variable or a protected input wrapper.
Do not put enrolment secrets in command arguments, filenames, CI output, or
shell history. Successful enrolment writes the device key and configuration
atomically and refuses to overwrite an existing identity.

After enrolment:

1. Remove the token from terminals, CI variables, and clipboard managers.
2. Start the node without the token.
3. Confirm its ID and online status in the control plane.
4. Verify discovered printers before exposing them.
5. Create and physically test profiles before publishing targets.

## Credential lifetime

A node's device key does not expire. Every request it makes is signed
individually, so there is no session or bearer token to refresh, and nothing
lapses on a timer that could take a node offline unattended.

Access ends in exactly one way: revoking the node in the control plane. The
next signed request it makes is rejected, and the node reports itself as
`unauthorized` rather than `offline` so the status distinguishes a revoked node
from a network fault.

## Rotating a device key

Rotate in place when a key may have been exposed but the node itself is still
trusted — a restored disk image, a departed administrator, a support bundle
that captured too much:

```sh
piqae-agent \
  --data-dir /secure/piqae \
  --control-plane-url https://api.piqae.example.com \
  --rotate-key
```

Rotation runs the same browser approval as first pairing and reuses the
installation ID recorded at pairing time, so the control plane rebinds the
existing node. The node ID, its printers, and any routing that points at them
survive. Approve the rotation from the workspace that already owns the node: if
the approval lands in a different workspace, the node refuses to replace its
local key and tells you so, leaving the original identity working.

The new key replaces the old one atomically. An interrupted rotation leaves
either the old key or the new one, never a partial file.

Nodes paired before installation IDs were recorded cannot rotate in place —
rotating would admit a second node and strand the first. Those nodes report the
problem and must be revoked and paired again.

## Clock synchronization

Signed requests carry a timestamp that the control plane checks against a
five-minute window. Every response, including a rejection, carries the server
clock, and a node applies the observed offset to its next request. A node with
a drifting clock therefore recovers on its own rather than failing until
someone investigates.

Keep time synchronization enabled anyway. Self-correction handles drift; it
does not handle a machine whose clock jumps unpredictably between requests.

Re-enrolment is not a casual repair step. Preserve local queue evidence and the
device identity during upgrades. Prefer `--rotate-key` over re-enrolment when
only the key needs replacing. Revoke a lost node server-side, then perform a
deliberate clean enrolment.

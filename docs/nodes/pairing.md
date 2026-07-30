# Pairing and enrolment

**Status:** browser pairing and one-time connected enrolment are implemented;
installer integration and platform release tiers vary.

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
then enter the eight-character code shown by the node. The device code is
single-use, expires after ten minutes, and is never placed in the URL or log.
The human web session is not retained by the node.

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

Re-enrolment is not a casual repair step. Preserve local queue evidence and the
device identity during upgrades. Revoke a lost node server-side, then perform a
deliberate clean enrolment.

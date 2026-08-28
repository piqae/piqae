# PrintPacket capability negotiation

`printpacket/v1` is the sole vendor-neutral packet identifier. Piqae is still
pre-release, so prior experimental identifiers are rejected instead of being
normalized, migrated, or advertised.

The renderer capability report is a set of independently testable facts. It is
not inferred from an app version. Negotiation version 2 reports exact packet
versions, feature IDs, conformance and output profiles, deterministic-output
support, input/output/page/resource limits, resource media types, direct-offline
support, and any reviewed printer-native language profiles. The implementation
version is diagnostic only.

The control plane intersects every required fact before registering a
`require_node` print and again before offering the job. A node which omits the
version 2 report is `unsupported_old_node`; standalone ABI strings do not
upgrade that result. Such a node never receives a PrintPacket descriptor.

Render readiness has three operator/developer states:

- `ready`: the exact node contract is currently compatible.
- `fallback_ready`: node rendering is incompatible, but the immutable,
  approved PDF can be delivered under the requested policy.
- `node_update_required`: the node is old or lacks a required packet feature,
  or `require_node` forbids the PDF fallback.

The response also includes `missing_features`, `supported_packet_versions`,
`current_implementation`, and `approved_pdf_fallback`. A Shopify, POS, or other
integrator can therefore identify the affected installation without parsing a
human message or guessing from an app version. `prefer_node` and `automatic`
store the exact fallback reason on the job.

## Printer-native output

`raw` is a transport shape, not a printer language. Native jobs must name both
an output profile and a language profile which the authenticated node has bound
to the exact printer ID. The server persists the resolved language, language
version, replay-tested support-pack profile version, media type, driver
fingerprint, support-pack digest, and printer ID. The complete tuple must match
the current authenticated report before reoffer and the node's latest local
support-pack/driver evidence before content materialization. Capability drift
or ID reuse therefore causes the server or node to withhold the offer.
Persisted native evidence is quarantined at every node process start and before
every inventory refresh. Only a complete successful discovery may atomically
publish a process-session/generation-bound profile set; a failed refresh or
binding-store fault withdraws RAW capability without disabling PDF delivery.

Every API surface requires this binding for printer-native bytes. A generic RAW
fallback is never substituted for a missing pinned profile.

`direct_offline` is only a reported execution fact. It does not authorize cloud
delivery, imply iOS background time, or prove output reached paper.

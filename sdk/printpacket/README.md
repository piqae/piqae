# `@printpacket/core`

Preview TypeScript contracts for the vendor-neutral `printpacket/v1` standard.
The package has no Piqae account or network dependency. Use `definePacket` for
typed templates, `preflightPacket` for fast bounded authoring feedback,
`requiredFeatures` for capability negotiation, and `renderCacheKey` for a
privacy-safe deterministic cache identity.

The bundled JSON Schema and a conforming renderer remain authoritative. The
preflight helper is intentionally not described as a renderer certification.

```ts
import { definePacket, preflightPacket } from "@printpacket/core";

const label = definePacket({
  format: "printpacket/v1",
  media: { kind: "label", width_mm: 50, height_mm: 30 },
  body: [{
    type: "barcode",
    value: { type: "path", path: ["sku"] },
    symbology: "code128",
    width_mm: 35,
    height_mm: 10
  }]
});

preflightPacket(label); // throws before an API or native-SDK call on bad shape
```

This package remains marked private until the independent PrintPacket package
scope, repository, changelog, provenance, and publication ownership are set up.
It is already usable as a monorepo workspace package.

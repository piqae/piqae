# Node guides

- [macOS](macos.md)
- [Windows](windows.md)
- [Headless Linux](linux-headless.md)
- [Pairing](pairing.md)
- [Updates](updates.md)
- [Diagnostics](diagnostics.md)
- [Standalone and embedded hosts](embedded-sdk.md)

## One runtime, two product forms

Every supported desktop/mobile platform is intended to expose the same durable
node in two forms:

- a **standalone node** owns the device installation, presents printer,
  profile, history and connection management, and accepts zero to many
  user-approved hosted or self-hosted connections;
- an **embedded node** is app-scoped and may use host-managed invitations from
  an integrator backend. It still supports zero to many isolated connections;
  “usually one” is a product default, not a queue or schema restriction.

An embedded desktop app uses `prefer_installed` by default. It attaches only
after the operating system verifies its application principal and the user
approves the requested capabilities. Otherwise it uses its own sandbox when
fallback was configured. `require_installed` never silently creates a second
queue, and `isolated_application` never opens the installed node's state.

The local node identity is display metadata: a privacy-safe OS device name,
optional site/location, and explicit labels. It never infers or uploads the
logged-in username or an address. Standalone macOS and Windows nodes expose a
local editor; each connected workspace may retain a separate cloud-side name
override without changing the installation identity or printer routes.

Always read the repository release support matrix before treating source
implementation as production support.

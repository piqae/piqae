# White-label and integrator product UX

This guide is for SaaS products, marketplaces, fulfilment systems, design tools,
and embedded applications that use Piqae for customer printing while keeping
their own brand and account model.

**Status:** the platform-account and multi-connector foundations are implemented
as Preview. Their production support claim remains Disabled until the platform
release-evidence gates pass. Product teams may build and evaluate the UX now,
but must not present the integration as Supported fleet infrastructure yet.

## Keep four identities separate

| Layer                   | Owned by                                                    | What the customer sees                             |
| ----------------------- | ----------------------------------------------------------- | -------------------------------------------------- |
| Integrator user/session | Your application                                            | Your normal sign-in, roles, and customer selector  |
| Customer account        | Your database, mapped to one isolated Piqae workspace       | Your customer or site name                         |
| Connector grant         | The customer/operator on a physical node                    | Which service may use which local printers         |
| Node and printers       | The durable Piqae installation and operating-system drivers | Computer, printers, profiles, readiness, and queue |

Do not ask a customer to understand Piqae Cloud, a self-hosted control plane,
workspace IDs, environments, connector IDs, or platform credentials. Those are
implementation and audit concepts. A self-hosted integrator, Piqae Cloud, and a
white-label child account use the same product model.

Recommended customer language:

```text
Connect this computer
Allow Acme Shipping to use printers on this computer.

(•) All current and future printers
( ) Only selected printers

[Cancel] [Allow printers]
```

Brand the requesting service (`Acme Shipping`) from verified connector metadata,
not from a URL query or browser-supplied string. Piqae may appear in legal,
security, open-source attribution, diagnostics, and advanced support material;
it does not need to dominate the normal workflow.

## Recommended application surfaces

### 1. Printing setup

Place printing inside the customer/site settings already used by your product:

```text
Printing
Connected computers                         2
Available printers                          5
Attention required                          1

[Connect a computer]
```

“Connect a computer” creates a short-lived connect session on your trusted
backend. Open its returned URL in the browser. Never expose the platform key or
construct an enrolment URL in browser JavaScript.

After the native node proves its installation identity, show consent locally on
that computer. The user chooses all printers or a selected subset there. Do not
add a second global “cloud access” switch: the connector grant is authoritative.

### 2. Printer picker

Show only printers projected into the signed-in customer account and selected
environment. A useful compact row contains:

```text
Production Labels
OKI Pro1050 · Labels PC · Ready
80 mm matte, black mark · Stock loaded
```

Use stable IDs behind the UI. Display names, queue names, driver names, and
profile names are not tenant or device identity. Never accept a printer ID from
the browser without resolving it again through the authenticated account.

For simple printers, selecting a printer and its default profile can be enough.
For professional workflows, prefer a logical target such as “80 mm product
labels” whose readiness includes an immutable profile revision and stock.

### 3. Node details and Queue

Use one provider-neutral node view across every hosting model:

- node health, version, pause/update state, and last contact;
- visible printers and their live readiness;
- recent, active, retained and uncertain jobs;
- connector summaries such as connected/attention/revoked and all/selected
  printer access;
- reauthorise or manage-printer-access actions when a verified URL is available;
- explicit reprint as a new job with a new audit record and idempotency key.

Do not group the queue by “Piqae SaaS” versus “self-hosted server.” Group it by
customer intent and operational state: printing, held, attention required,
completed, cancelled, uncertain. A connector origin can appear in job details or
filters when more than one authorised service sends work to the same physical
node, but one tenant must never learn another tenant's name, jobs, or existence.

The native node's local Queue can aggregate its own connector activity because
the local operator controls that computer. Each remote integrator view remains
strictly tenant-scoped. This is how one physical node can serve several SaaS or
self-hosted systems without producing a provider-specific dashboard.

### 4. Recovery and permissions

Prefer actions over protocol details:

| State                             | Customer action                           |
| --------------------------------- | ----------------------------------------- |
| Node offline                      | “Check the printer computer”              |
| Connector needs authentication    | “Reconnect service”                       |
| Selected printer not granted      | “Update printer access”                   |
| Profile stale after driver change | “Review settings on Labels PC”            |
| Stock mismatch                    | “Load 80 mm matte stock”                  |
| Delivery uncertain                | “Check the printer before printing again” |

Reauthorisation must return through a short-lived, auditable server-created
session. Do not embed a permanent bearer in a deep link. If the API does not
provide a verified manage URL, show state and guidance rather than inventing a
provider URL.

## Sub-account mapping

Use one immutable external ID per customer account in your system:

```text
integrator customer org_01JQ8K8M6Q
          ↓ trusted server mapping
Piqae workspace wsp_... with Test and Live environments
```

Resolve this mapping after authenticating the user and checking their role in
your application. The browser may request “print order 10428”; it may not choose
`workspace_id`, `environment_id`, or platform context. Your backend chooses Test
or Live from business policy and creates the account-scoped SDK client.

For agencies or resellers with customers below them, keep the hierarchy in your
own authorization model. Create a Piqae account for each isolation boundary that
must have separate printers, jobs, webhooks, quotas, audit history, and keys. Do
not flatten unrelated child customers into one workspace merely because they
share a reseller.

## Recommended backend-for-frontend flow

```text
signed-in user
  → integrator authorizes customer + operation
  → immutable external customer ID
  → platform account get-or-create
  → server selects Test or Live
  → account-scoped SDK client
  → printers / targets / uploads / jobs / webhooks
```

Return a product-shaped view model to browser or mobile clients. Useful fields
include your own display label plus Piqae resource ID, state, readiness reason,
profile summary, stock summary, node label, and permitted actions. Never return
the platform key, node identity keys, enrolment token, upload authorization,
connector signing keys, or another connector's data.

## Branding and support boundaries

- Your application owns customer navigation, terminology, help links, and
  authentication.
- The installed node may be Piqae-branded or distributed under an integrator's
  permitted branding, but its identity proof and connector consent cannot be
  bypassed.
- Preserve Apache-2.0 notices and do not hide security or diagnostic provenance
  needed by operators.
- Use your support contact first, then include bounded Piqae node diagnostics
  when escalation is required.
- Say “job accepted” or “reported complete” according to actual state; never
  convert spooler handoff into “printed successfully.”

## Minimum launch checklist

1. One immutable external ID and isolated account per customer boundary.
2. Platform credential exists only in a trusted secret manager.
3. Test and Live selection is server-controlled.
4. Connect and reauthorise sessions are short-lived and auditable.
5. Consent clearly names the requesting service and all-versus-selected grant.
6. Printer picker uses current account-scoped resources and stable IDs.
7. Queue exposes uncertain delivery and makes reprint a new confirmed action.
8. Webhooks are signature-verified and deduplicated.
9. Revoking one connector cannot affect other tenants on the physical node.
10. Support language matches the checked-in support matrix.

See [Platform accounts](platform-service-accounts.md),
[multi-integrator node connectors](multi-integrator-node-connectors.md), and the
[web design platform integration](web-design-platform-integration.md) for the
API and lifecycle details behind this product model.

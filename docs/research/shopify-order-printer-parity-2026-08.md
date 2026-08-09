# Shopify order-document app: verified parity and implementation contract

Research date: 2026-08-09. Target Shopify API version: `2026-07`.

This document is the implementation contract for the Shopify application. It
separates verified competitor behavior, Shopify platform requirements, and
planned Piqae behavior. A row is not a support claim until its acceptance tests
pass and the release support matrix enables it.

## Sources and evidence policy

Order Printer Pro (OPP) facts below come from its current [Shopify App Store
listing](https://apps.shopify.com/order-printer-pro), [official product
site](https://get.orderprinterpro.com/), [vendor comparison/help
article](https://help.forsbergplustwo.com/en/articles/3921882-difference-between-orderlyprint-and-order-printer-pro),
and [vendor 2026 product roundup](https://get.orderprinterpro.com/blogs/app-updates/product-roundup).
Shopify requirements come only from current Shopify developer documentation.
Marketing articles are treated as evidence of advertised behavior, not proof of
its reliability or internal implementation. We must not describe Piqae as
faster, more accurate, fully compatible, compliant with a tax regime, or 1:1
until a corresponding, reproducible test supports that statement.

## Verified pricing baseline

Match these public OPP tiers exactly at launch unless Shopify's listing changes:

| Plan    | Monthly orders |        Price | Trial          |
| ------- | -------------: | -----------: | -------------- |
| Free    |           0-50 |         Free | Not applicable |
| Starter |         51-500 | USD 10/month | 14 days        |
| Growth  |      501-5,000 | USD 20/month | 14 days        |
| Scale   |         5,001+ | USD 40/month | 14 days        |

Every listed tier advertises bulk print/export, draft-order print/export,
automated PDF invoices, customizable templates, and product, variant, customer,
order, and draft-order metafields. The app listing also advertises an in-app
Peppol/e-invoicing add-on but does not publish its price; it is therefore a
separate future capability, not silently included in parity pricing.

Use [Shopify App Pricing](https://shopify.dev/docs/apps/launch/billing/shopify-app-pricing),
not a parallel card checkout. Shopify hosts plan selection and billing. For a
new app after 2026-04-28, confirm subscription state with the Partner API and
`plan_handle`; do not design around the discontinued subscription-change
webhook or legacy `charge_id`. A merchant who supplies an existing Piqae
subscription chooses a free "Connect Piqae" entitlement inside the embedded
app; Shopify-paid child accounts remain confined to this Shopify integration.
This entitlement distinction must not bypass the 50-order limit unless the
external Piqae account is successfully linked and authorized.

Order counting contract: count distinct Shopify orders first rendered, printed,
downloaded, or exposed through an automated document link in the shop's current
billing interval. Reprints and additional template variants for the same order
do not increase the count. Draft orders remain included in plan capability but
do not count until they become an order. This is a Piqae product rule and must
be stated on the pricing screen because OPP's public pages do not disclose its
exact metering algorithm.

## Feature parity matrix

Priority means required for the named release gate, not current support.

| Area                    | Verified OPP behavior                                                                                                                                                                   | Piqae implementation requirement                                                                                                                                                                                                                      | Priority          |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------- |
| Documents               | Invoice/receipt, gift receipt, credit note/refund, quote/pro forma, draft order, delivery note/packing slip, returns form; vendor help also lists purchase order and commercial invoice | Ship tested starter templates for every named type. Each is a versioned Piqae template revision and uses one normalized order snapshot                                                                                                                | GA parity         |
| Single order            | Print/export from Shopify admin                                                                                                                                                         | Order detail Admin print action plus embedded preview; destination defaults to last usable Piqae printer, with browser/PDF always available                                                                                                           | BFS/P0            |
| Bulk orders             | Filter, bulk print and bulk PDF export; vendor help historically describes up to 50 per operation                                                                                       | Order index selection print action. Preserve selected GIDs, show per-item progress, stream ready items, retry individual failures, and never duplicate a physical job                                                                                 | BFS/P0            |
| Draft orders            | Print/export quotes and invoices                                                                                                                                                        | Draft detail and draft index selection actions using Admin action targets; immutable draft snapshot and `read_draft_orders`                                                                                                                           | P0                |
| Direct printing         | OPP uses Shopify/system printing; public sources do not establish a PrintNode-style route                                                                                               | Prefer a linked Piqae node/printer. Confirm template, printer, media, copies, and selected IDs before enqueue. Browser print/download is the fallback, never a hidden failure fallback                                                                | Differentiator/P0 |
| PDF                     | Generate, download, bulk download, print/export, configurable filenames                                                                                                                 | Reuse the exact immutable rendered artifact for preview, download and print. ZIP multiple PDFs or explicitly merge; filename pattern is validated and path-safe                                                                                       | P0                |
| Automated delivery      | Permanent/static PDF links insertable into notifications, customer account and order-status surfaces                                                                                    | Signed opaque link resolving an immutable authorized artifact; merchant chooses template/events/locale. Customer-account block supplies native download action. Revocation and retention are explicit                                                 | P0                |
| Email                   | Automatic PDF invoices, receipts and quotes; public evidence is primarily PDF links placed in Shopify notifications                                                                     | P0 is secure PDF-link insertion instructions/extension, not an unsupported claim that Shopify notification APIs attach arbitrary files. Optional transactional sending requires a separately consented email provider and delivery/audit status       | P0 link; P1 email |
| Admin/mobile            | Shopify Admin, Shopify mobile                                                                                                                                                           | Embedded responsive App Home, App Bridge session-token auth, Admin extensions using Shopify web components                                                                                                                                            | BFS/P0            |
| POS                     | Works in Shopify POS; invoices/receipts/documents                                                                                                                                       | POS smart-grid tile and contextual draft-order actions. On API `2026-07`, use `shopify.printing`: direct connected receipt printers accept HTML/images; PDF uses system dialog. Piqae-node delivery is a separately labeled destination               | P0                |
| Templates               | Customizable Liquid/code templates, compatible with Shopify Order Printer templates; logo/color/font/branding/fields                                                                    | Visual bounded editor produces `piqae.document/v1`; advanced Liquid compatibility mode is sandboxed and compiled to normalized Piqae data/document operations. Import reports exact unsupported constructs and never claims silent 100% conversion    | P0/P1             |
| QR/barcodes             | QR and barcode customization advertised                                                                                                                                                 | Vector QR is P0 and decoded during preflight. Common linear barcodes need symbology-specific golden/decode tests before enablement                                                                                                                    | QR P0; barcode P1 |
| Numbering               | Invoice numbers, sequential numbering, file naming                                                                                                                                      | Tenant-scoped, transactional, gap-policy-documented number allocation; never infer invoice issue date from order number; credit notes reference original invoice                                                                                      | P0                |
| Brand/layout            | Logo, colors, fonts, fields, customizable designs                                                                                                                                       | Guided setup from shop brand settings plus page presets A4/A5/Letter/receipt. Fonts/assets pinned and embedded; missing assets fail visibly                                                                                                           | P0                |
| Metafields              | Order, draft order, customer, product, variant                                                                                                                                          | Merchant allowlist of definitions/namespace-key pairs; query only required fields. Typed values normalized deterministically; missing/denied fields produce template diagnostics                                                                      | P0                |
| Multi-currency          | Supported                                                                                                                                                                               | Preserve shop and presentment money sets, ISO code, rounding, discounts, duties, tips and refunds. Template explicitly chooses transaction/presentment/shop values                                                                                    | P0                |
| Translation             | Documents multi-language; current app UI advertised in 21 languages                                                                                                                     | Separate merchant UI locale from document locale. Resolve order/customer/market preference, then shop default. Template translation catalog has explicit fallback and RTL tests. Do not promise 21 UI locales until each is shipped                   | P0 documents      |
| Tax/VAT                 | Tax calculation, VAT and taxes advertised                                                                                                                                               | Print Shopify's authoritative tax lines, prices-including-tax state, exemptions and registration text configured by merchant. Never implement a competing tax engine or advertise legal compliance by default                                         | P0                |
| B2B                     | B2B supported; vendor documents PO-number support                                                                                                                                       | Normalize `purchasingEntity` company/location, company tax IDs where accessible, purchase order, payment terms, billing/shipping contacts and tax exemptions. Degrade explicitly on non-Plus/no-B2B data                                              | P0                |
| Refunds                 | Refund/credit-note documents                                                                                                                                                            | Subscribe to refund creation and order update; inspect transaction status because refund creation alone does not prove money moved. Allocate credit-note number only under merchant policy and link original invoice                                  | P0                |
| Edited/cancelled orders | Needed for correct operational documents though not fully enumerated on listing                                                                                                         | Snapshot `updatedAt`; regenerate creates a new artifact/revision, never mutates an issued invoice. Display cancelled, removed, added, refunded and fulfilled quantities                                                                               | P0 correctness    |
| Automation              | Automated invoices/static links                                                                                                                                                         | Rules by Shopify event, document, template, locale, destination and printer. Durable idempotency is `(shop,event,resource,rule revision)`. Default automation generates links; unattended physical printing is opt-in with printer and failure policy | P0/P1             |
| Customer accounts       | App listing says customer accounts                                                                                                                                                      | `customer-account.order-status.block.render` download action for new customer accounts. Unauthenticated public order-status pages do not support extensions; pre-auth access requires protected-customer-data approval                                | P0                |
| Security                | Data security advertised                                                                                                                                                                | Encrypt tokens, snapshots and artifacts; tenant-fence every operation; short opaque links; audit access; redact on mandatory webhooks; no Liquid network/file/code execution                                                                          | P0                |
| E-invoicing             | Peppol add-on advertised                                                                                                                                                                | Separate jurisdiction/provider project requiring structured semantic invoice data, network/provider certification and country-specific acceptance tests. A PDF is not a Peppol invoice                                                                | Post-GA           |

## Shopify-native surface architecture

Follow the current [Built for Shopify requirements](https://shopify.dev/docs/apps/launch/built-for-shopify/requirements)
and [App Store requirements](https://shopify.dev/docs/apps/launch/shopify-app-store/app-store-requirements):

1. App Home is embedded, uses the latest App Bridge script before other scripts,
   session-token authentication, Shopify web components/Polaris conventions,
   responsive layouts and no second sign-up.
2. All new public-app Admin access is GraphQL-only. Pin `2026-07`, test the next
   release candidate, and keep query documents versioned.
3. The mandatory invoice/receipt category integration is both
   `admin.order-details.print-action.render` and
   `admin.order-index.selection-print-action.render`. These are required even
   though Piqae direct printing is the preferred destination.
4. Use draft-order action targets because Shopify does not list corresponding
   draft-order print targets:
   `admin.draft-order-details.action.render` and
   `admin.draft-order-index.selection-action.render`.
5. Use the print action only for the print workflow. Put printer setup, account
   connection, templates, automation, numbering and billing in embedded App
   Home. Extensions must not contain promotions or review requests.
6. Add a POS tile/modal and draft-order contextual actions. The 2026-07
   [Printing API](https://shopify.dev/docs/api/pos-ui-extensions/latest/target-apis/platform-apis/printing-api)
   can print HTML/images directly to a discovered receipt printer; PDF cannot be
   sent directly to that hardware API and must use the system dialog. Do not
   describe a Piqae node as a Shopify POS hardware printer.
7. Add `customer-account.order-status.block.render` with a native download
   button for new customer accounts. Merchant placement and protected customer
   data approval are release dependencies.

Built for Shopify also requires at least 50 net installs on active paid-plan
shops, five reviews, the current rating threshold, and measured App Home Web
Vitals at p75: LCP <= 2.5 s, CLS <= 0.1, INP <= 200 ms. These cannot be certified
from local tests. Record actual Shopify assessment evidence before applying.

## Access scopes

Start with least privilege and request expanded protected-order access only when
retention beyond Shopify's default order window is necessary:

- `read_orders`: individual/bulk order documents and order print actions.
- `read_draft_orders`: quotes, draft invoices and draft metafields.
- `read_products`: product/variant fields and metafields used by published
  templates. Avoid this scope if a merchant's templates do not require it only
  if Shopify permits optional scopes for the final architecture.
- `read_customers`: customer fields/metafields and B2B document parties. This is
  protected customer data and requires the corresponding Partner approval.
- `read_companies`: verify availability and approval in the pinned schema before
  requesting; company information reachable from orders should be preferred
  when sufficient.
- `read_payment_terms`: required by Shopify for `PaymentTerms`; request it when
  B2B/draft payment schedules are enabled.

Do not request write scopes for a read/render/print product. POS and Admin
extensions authenticate their backend calls with the Shopify-provided token;
the backend validates issuer, audience, destination/shop and expiry and maps the
staff/shop to a tenant. Offline tokens stay encrypted server-side.

## Versioned GraphQL data contract

Queries are server-owned persisted documents. Extension input is a list of GIDs,
never raw query text. Fetch bounded pages and use GraphQL bulk operations only
for a merchant-requested/reconciliation workload where their asynchronous
semantics are appropriate.

The order query must include, when present and approved:

```graphql
query OrderDocument(
  $id: ID!
  $lineItems: Int!
  $metafields: [HasMetafieldsIdentifier!]!
) {
  shop {
    id
    name
    email
    currencyCode
    taxesIncluded
    billingAddress {
      address1
      address2
      city
      provinceCode
      zip
      countryCodeV2
    }
  }
  order(id: $id) {
    id
    name
    createdAt
    updatedAt
    processedAt
    cancelledAt
    cancelReason
    displayFinancialStatus
    displayFulfillmentStatus
    test
    currencyCode
    presentmentCurrencyCode
    subtotalPriceSet {
      shopMoney {
        amount
        currencyCode
      }
      presentmentMoney {
        amount
        currencyCode
      }
    }
    totalDiscountsSet {
      shopMoney {
        amount
        currencyCode
      }
      presentmentMoney {
        amount
        currencyCode
      }
    }
    totalShippingPriceSet {
      shopMoney {
        amount
        currencyCode
      }
      presentmentMoney {
        amount
        currencyCode
      }
    }
    totalTaxSet {
      shopMoney {
        amount
        currencyCode
      }
      presentmentMoney {
        amount
        currencyCode
      }
    }
    totalPriceSet {
      shopMoney {
        amount
        currencyCode
      }
      presentmentMoney {
        amount
        currencyCode
      }
    }
    currentTotalPriceSet {
      shopMoney {
        amount
        currencyCode
      }
      presentmentMoney {
        amount
        currencyCode
      }
    }
    note
    tags
    poNumber
    billingAddress {
      name
      company
      address1
      address2
      city
      province
      provinceCode
      zip
      country
      countryCodeV2
      phone
    }
    shippingAddress {
      name
      company
      address1
      address2
      city
      province
      provinceCode
      zip
      country
      countryCodeV2
      phone
    }
    customer {
      id
      displayName
      email
      phone
      locale
      metafields(identifiers: $metafields) {
        namespace
        key
        type
        value
      }
    }
    purchasingEntity {
      ... on PurchasingCompany {
        company {
          id
          name
        }
        location {
          id
          name
        }
      }
      ... on Customer {
        id
        displayName
      }
    }
    paymentTerms {
      paymentTermsName
      dueInDays
      paymentSchedules(first: 50) {
        nodes {
          dueAt
          amount {
            amount
            currencyCode
          }
        }
      }
    }
    taxLines {
      title
      rate
      ratePercentage
      priceSet {
        shopMoney {
          amount
          currencyCode
        }
        presentmentMoney {
          amount
          currencyCode
        }
      }
    }
    shippingLines(first: 50) {
      nodes {
        title
        code
        discountedPriceSet {
          shopMoney {
            amount
            currencyCode
          }
          presentmentMoney {
            amount
            currencyCode
          }
        }
        taxLines {
          title
          rate
          priceSet {
            shopMoney {
              amount
              currencyCode
            }
            presentmentMoney {
              amount
              currencyCode
            }
          }
        }
      }
    }
    lineItems(first: $lineItems) {
      nodes {
        id
        name
        title
        variantTitle
        sku
        quantity
        currentQuantity
        refundableQuantity
        originalUnitPriceSet {
          shopMoney {
            amount
            currencyCode
          }
          presentmentMoney {
            amount
            currencyCode
          }
        }
        discountedTotalSet {
          shopMoney {
            amount
            currencyCode
          }
          presentmentMoney {
            amount
            currencyCode
          }
        }
        taxLines {
          title
          rate
          priceSet {
            shopMoney {
              amount
              currencyCode
            }
            presentmentMoney {
              amount
              currencyCode
            }
          }
        }
        customAttributes {
          key
          value
        }
        product {
          id
          title
          metafields(identifiers: $metafields) {
            namespace
            key
            type
            value
          }
        }
        variant {
          id
          title
          barcode
          metafields(identifiers: $metafields) {
            namespace
            key
            type
            value
          }
        }
      }
    }
    fulfillments(first: 50) {
      id
      status
      createdAt
      trackingInfo {
        company
        number
        url
      }
      fulfillmentLineItems(first: 250) {
        nodes {
          quantity
          lineItem {
            id
          }
        }
      }
    }
    refunds {
      id
      createdAt
      note
      totalRefundedSet {
        shopMoney {
          amount
          currencyCode
        }
        presentmentMoney {
          amount
          currencyCode
        }
      }
      refundLineItems(first: 250) {
        nodes {
          quantity
          restockType
          lineItem {
            id
          }
          subtotalSet {
            shopMoney {
              amount
              currencyCode
            }
            presentmentMoney {
              amount
              currencyCode
            }
          }
        }
      }
      transactions(first: 50) {
        nodes {
          id
          status
          kind
          processedAt
          amountSet {
            shopMoney {
              amount
              currencyCode
            }
            presentmentMoney {
              amount
              currencyCode
            }
          }
        }
      }
    }
    metafields(identifiers: $metafields) {
      namespace
      key
      type
      value
    }
  }
}
```

This is a field contract, not a promise the hand-written sample validates
unchanged forever. Schema/codegen validation against `2026-07` decides exact
field spellings and pagination. In particular, tax fields and connection
arguments must be generated from the pinned schema rather than copied blindly.
Paginate all potentially unbounded connections; never truncate a legal document
silently. The draft query needs equivalent money, line item, address, customer,
payment terms, purchase order and allowlisted metafield data from `draftOrder`.

## Webhook and reconciliation contract

Declare app-specific subscriptions in `shopify.app.toml` at `2026-07`:

- `app/uninstalled`, `app/scopes_update`;
- `orders/create`, `orders/updated`, `orders/cancelled`, `orders/paid`,
  `orders/fulfilled`, `orders/partially_fulfilled`;
- `refunds/create`;
- `draft_orders/create`, `draft_orders/update`, `draft_orders/delete`;
- mandatory `customers/data_request`, `customers/redact`, `shop/redact`.

All deliveries: authenticate HMAC over the raw request body, validate topic/shop,
deduplicate by webhook ID, persist a compact inbox record, return 2xx quickly,
then process asynchronously. Use event/resource timestamps because Shopify warns
webhooks can arrive out of order. Webhooks are invalidation/automation triggers,
not the authoritative invoice payload: refetch the affected resource through
GraphQL before rendering. Reconcile active orders/drafts periodically because
delivery alone is not a complete synchronization guarantee.

Mandatory privacy behavior follows Shopify's [privacy-law compliance
contract](https://shopify.dev/docs/apps/build/compliance/privacy-law-compliance):
invalid HMAC returns 401; valid requests get 2xx; subject access/redaction is
completed within 30 days unless legally retained. `shop/redact` removes the
shop's data after the prescribed uninstall lifecycle. Legal invoice retention
must be configured and documented by the merchant; redact customer identity
where retention law permits/requires rather than keeping an unexplained copy.

## Print and artifact flow

```text
Admin/POS/customer-account extension
  -> authenticated Shopify BFF command
  -> refetch and normalize Shopify resource
  -> immutable template revision + locale + normalized snapshot
  -> Piqae render registration (stable idempotency key)
  -> one immutable PDF artifact and preflight evidence
  -> preferred: explicit Piqae printer job
     fallback: Shopify/browser print surface or authenticated PDF download
```

No physical action occurs from a preview, hover, extension mount, webhook retry,
or automatic fallback. A direct print command names the printer and expected
document count. Render completion is not print completion; retain Piqae's
accepted/printing/reported-complete/uncertain distinction. Automation that
physically prints is separately enabled and presents its retry/uncertain policy.

For POS native receipt hardware, provide bounded same-origin HTML or image to
`shopify.printing.print(src, {printer})`. For Piqae nodes, enqueue through the
Piqae API. For PDF or when no direct printer is selected, use the system dialog
or download. These are three different delivery mechanisms in product copy and
telemetry.

## Acceptance gates

Parity is complete only when the corpus covers:

- each starter document type across order, draft, cancelled, edited, partially
  fulfilled, fully/partially refunded, gift, free, tax-inclusive and tax-exempt;
- shop/presentment currency divergence, zero/negative adjustments, duties,
  discounts and rounding;
- B2B company/location, PO number and payment schedules;
- Latin, CJK, Arabic/RTL, emoji fallback and long unbroken text;
- 1, 50, 250 and greater-than-250 line items with complete pagination;
- missing/deleted products, variants and customers;
- QR decode, filename safety, template resource bounds and hostile Liquid/input;
- single and bulk Admin print targets, mobile Admin, POS iOS/Android, customer
  account authenticated/pre-authenticated states;
- node unavailable/offline/reconnect, ambiguous delivery and explicit PDF
  fallback without duplicate printing;
- webhook duplicates, reordering, delay, deletion and reconciliation repair;
- plan boundaries, trials, external-account linking/unlinking and uninstall/
  reinstall isolation.

Built for Shopify application evidence additionally requires live install/review
thresholds and Shopify-measured Web Vitals. Those gates cannot be replaced by
unit tests or a local Lighthouse result.

## Deliberate non-claims and open dependencies

- Exact Liquid/HTML compatibility is not established. Import is diagnostic and
  adapters declare their compatibility.
- Piqae does not calculate tax or provide legal/tax advice; it renders Shopify
  and merchant-provided facts.
- Peppol/e-invoicing is not PDF rendering and is not initial parity.
- POS direct receipt printing and Piqae node printing are not interchangeable.
- Shopify public evidence does not prove OPP's internal batch limits, latency,
  metering details, delivery guarantees, or generated PDF correctness. Do not
  encode assumptions about them as competitor claims.
- Protected customer data approval, development/Plus stores, Shopify Partner
  configuration, live POS devices and the install/review thresholds are external
  release dependencies.

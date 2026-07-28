# PrintNode capability inventory

## Purpose and research boundary

This is a functional inventory derived from PrintNode's public documentation,
FAQ, download page, public API reference, and open-source SDKs. It is an
independent summary, not a copy of PrintNode's documentation. The source
snapshot was reviewed on 29 July 2026; links are in [references.md](references.md).

PrintNode's private client/server wire protocol and proprietary printing
backends are outside the research boundary. Compatibility work should remain a
clean-room implementation based on public behavior.

## How PrintNode works

The public architecture has four relevant actors:

1. The integrating application calls the hosted JSON API.
2. The PrintNode server registers and queues the job.
3. A PrintNode Client maintains a network connection from a computer that can
   access the target printer.
4. The client downloads or receives content and submits it to the
   operating-system print queue using locally installed drivers.

The API responds to job creation after registering/enqueueing the job; it does
not wait for the printer. The client automatically synchronises installed
printers and their driver-reported capabilities to the account.

PrintNode therefore has at least two queues:

- a server-side delivery queue, which retains a job until a client receives it
  or `expireAfter` is reached; and
- the operating-system spooler queue, which owns the job after client
  submission.

The documented stable state `done` means the client delivered the job to the
operating-system queue. PrintNode explicitly says that the job can still fail
after this state. This distinction must be preserved in compatibility mode and
improved in the native API.

## Supported environments

The documented client runs on:

- Windows;
- macOS;
- common Linux distributions;
- Raspberry Pi and other low-power Linux devices;
- a headless Linux/Chromebook environment with caveats.

PrintNode does not currently advertise Android or iOS clients. Printers must be
installed in the host operating system. The client detects installed queues; it
does not independently discover every printer on the network.

On Windows the client can run as an interactive desktop/tray application or a
Windows Service. Service mode avoids duplicate instances in multi-user
sessions, but runs as `LocalSystem` by default, so user-specific defaults and
network-printer access can differ. A local web interface is available for the
service.

The July 2026 public downloads are large relative to the low-resource goal
(approximately 104 MB for Windows and 156 MB for the current Apple Silicon
macOS package). PrintNode's feature page claims roughly 40 MB Windows memory
usage. These are reference points, not independently verified measurements.

## Printer coverage

PrintNode's core compatibility claim is based on using installed operating
system drivers: if the operating system can print to a configured printer, the
client should normally be able to print to it.

The public printer list is a record of devices customers have used, not a
driver database or exclusive allow-list. It contains conventional office
printers, label printers, receipt printers, virtual printers, USB devices, and
network queues. Special setup guides exist for Zebra and DYMO devices.

## Content modes

`POST /printjobs` supports:

| `contentType` | Meaning |
| --- | --- |
| `pdf_uri` | Client downloads a PDF from a URI. |
| `pdf_base64` | PDF bytes are base64-encoded in the API request. |
| `raw_uri` | Client downloads printer-language bytes from a URI. |
| `raw_base64` | RAW bytes are base64-encoded in the API request. |

URI content may use HTTP Basic or Digest authentication. URI mode can keep
document bytes away from the PrintNode server if the client fetches directly.
Base64 mode passes content through PrintNode. PrintNode says document content
is deleted after printing.

RAW printing bypasses normal document rendering and sends device-specific
languages such as ZPL, EPL, ESC/POS, PCL, or PostScript. Print options do not
apply to RAW content. Features such as a cutter or cash drawer are driven by
RAW commands.

## Print job fields

The documented create request includes:

| Field | Required | Function |
| --- | --- | --- |
| `printerId` | Yes | Destination printer identifier. |
| `contentType` | Yes | One of the four PDF/RAW URI/base64 modes. |
| `content` | Yes | URI or base64 document content. |
| `title` | No | OS queue job title. |
| `source` | No | Integration/origin description. |
| `options` | No | PDF driver/rendering options. |
| `expireAfter` | No | Maximum server retention while delivery is unavailable; documented default is 14 days. |
| `qty` | No | Number of separate submissions to the OS queue; required for repeated RAW jobs or drivers without copy support. |
| `authentication` | No | Basic or Digest credentials for URI retrieval. |

The `X-Idempotency-Key` header prevents a repeated create request from
producing a second print. A reused key is documented to return HTTP 409.

The FAQ provides an ordering guarantee only when the caller waits for the first
creation response before submitting the second job. Concurrent submissions can
be ordered differently.

## Print options

All documented options are optional and apply to rendered jobs:

| Option | Behavior and qualification |
| --- | --- |
| `bin` | Driver-reported input tray or output bin name. |
| `collate` | Collate multiple copies when supported. |
| `color` | Colour or grayscale; PrintNode documents a Windows backend limitation. |
| `copies` | Driver-level number of copies, bounded by reported capability. |
| `dpi` | Driver-reported resolution string such as `300x300`. |
| `duplex` | `long-edge`, `short-edge`, or `one-sided`. |
| `fit_to_page` | Scale the document to the selected page. Backend support varies. |
| `media` | Driver-reported medium, such as label or photo stock. |
| `nup` | Multiple logical pages per sheet; documented as macOS-only. |
| `pages` | PDF page set/range using print-dialog-like syntax. |
| `paper` | Driver-reported paper name. |
| `rotate` | Absolute 0, 90, 180, or 270 degree rotation; driver behavior varies. |

`qty` and `copies` are intentionally different. `copies` asks a single driver
job to produce copies; `qty` submits the document repeatedly.

## Printer capability model

Printer objects include their computer and:

- stable numeric identifier;
- name and description;
- default flag;
- creation timestamp;
- online/offline state;
- a capabilities object when available.

The documented capability object includes:

- `bins`: tray/output-bin names;
- `collate`: collation support;
- `color`: colour support;
- `copies`: maximum driver-supported copies;
- `dpis`: supported resolution strings;
- `duplex`: duplex support;
- `extent`: minimum and maximum page dimensions;
- `medias`: media names;
- `nup`: supported pages-per-sheet values;
- `papers`: mapping from paper name to width and height in tenths of a
  millimetre;
- `printrate`: optional rate and unit;
- `supports_custom_paper_size`.

This is a lossy common denominator over different native driver models. A clone
needs a richer internal capability representation, then a projection into this
compatibility shape.

## Public REST resources

### Identity, computers, and printers

- `GET /whoami`
- `GET /computers`
- `GET /computers/{computer-set}`
- `DELETE /computers`
- `DELETE /computers/{computer-set}`
- `GET /printers`
- `GET /printers/{printer-set}`
- `GET /computers/{computer-set}/printers`
- `GET /computers/{computer-set}/printers/{printer-set}`

A “set” is a comma-separated set of positive integer IDs.

### Print jobs

- `POST /printjobs`
- `GET /printjobs`
- `GET /printjobs/{print-job-set}`
- `GET /printers/{printer-set}/printjobs`
- `GET /printers/{printer-set}/printjobs/{print-job-set}`
- corresponding `DELETE` forms for cancellation;
- `GET /printjobs/states`
- `GET /printjobs/{print-job-set}/states`

Cancellation is only guaranteed before delivery to the client. Completed or
already delivered jobs cannot be cancelled through the PrintNode API.

### Pagination and errors

Collection pagination uses:

- `limit`, defaulting to 100;
- `dir=asc|desc`;
- `after={id}`.

Useful response headers include `Request-Id`, API/account/authentication
information, record counts, and response timing. The documented rate limit is
10 requests per second per account with some burst tolerance.

Error bodies use a short `code`, a human-readable `message`, and in some cases a
`uid` matching `Request-Id`.

### Webhook, account, download, and utility resources

The remaining documented management surface includes:

- `GET /webhooks`, `POST /webhook`, and `PATCH`/`DELETE
  `/webhook/{webhook-id}`;
- `POST`/`PATCH`/`DELETE /account` for child account management;
- `GET`/`PUT /account/state`;
- `GET`/`POST`/`DELETE /account/tag/{name}`;
- `GET`/`POST`/`DELETE /account/apikey/{description}`;
- `GET /account/controllable`;
- `GET /client/key/{uuid}` for delegated client enrolment;
- `GET /download/client/{operating-system}`;
- `GET /download/clients` and `GET`/`PATCH
  `/download/clients/{download-ids}`;
- `GET /ping`;
- `GET /noop`.

Some account, client-key, and download operations are only meaningful to
Integrator Accounts. They are compatibility-later features rather than part of
the reliable print MVP.

## Job states

The stable documented states are:

| State | What it proves |
| --- | --- |
| `new` | Server registered the job. |
| `sent_to_client` | Server sent the job to a client. |
| `done` | Client handed the job to the OS print queue; physical output is not guaranteed. |
| `error` | An error occurred while attempting client execution. |
| `expired` | Client delivery did not happen before expiry. |

PrintNode may record other internal states, but only stable states are promised
through webhooks. State records include job ID, state, message, data,
client version, timestamp, and age relative to the initial state.

## Webhooks and live data

Webhooks support at least:

- `computer state`;
- `print job state`;
- `*` for all supported message types.

Events are queued per target. The public documentation describes immediate
delivery after success, a five-second delay after failure, and only one retry
before dropping an event. A successful receiver must return a 2xx response and
the expected acknowledgement header. Up to five webhooks per account are
documented.

Webhooks are batched as an array and include event type, account,
controlling account, timestamp, and type-specific data. The configuration
includes URL, secret, and selected message types.

The browser JavaScript SDK exposes an RFC 6455 WebSocket API. Its documented
live functionality includes scale measurements and client connection tracking.
Scale subscriptions can filter by computer, device name, and device number.

## Scales

PrintNode supports:

- USB HID weighing scales;
- serial/RS-232 scales;
- serial-to-USB devices;
- a virtual test scale for development.

The client detects HID devices and streams readings to the server. Serial scale
protocols require model-specific support and local configuration.

REST endpoints can list scales on a computer or read a selected device.
Measurements normalise mass to micrograms and also expose display units,
resolution, USB vendor/product identifiers, port, timestamps, device name, and
device number. Recent readings are ephemeral; the public test documentation
describes a roughly 45-second window.

Scale support is a meaningful parity project of its own. It should reuse the
agent connection and event pipeline but remain outside the first reliable
printing milestone unless the organisation depends on it.

## Account and integration features

The public API and feature pages describe:

- API keys;
- account state and tags;
- Integrator Accounts;
- isolated Child Accounts controlled by an Integrator Account;
- acting as a child account using ID, email, or creator reference headers;
- child-account creation, modification, suspension, and deletion;
- delegated authentication through a customer endpoint;
- client keys;
- custom-branded client builds;
- downloadable client metadata;
- official PHP, Python, JavaScript, Ruby, and Java libraries;
- Make and Zapier integrations plus third-party commerce/ERP plugins;
- a commercial standalone/private deployment.

These are not needed to stop paying for PrintNode internally. They become
important for drop-in SaaS positioning.

## Commercial model snapshot

PrintNode counts one API print request as one print regardless of the document's
page count. A “computer” is a device running the connected client, regardless of
how many printers it exposes. Pricing is mutable; this snapshot is useful only
for a build-versus-buy model and must be refreshed before a business decision.

Public USD Single Account pricing observed on 29 July 2026:

| Plan | Price | Included use |
| --- | --- | --- |
| Lite | Free | 50 prints/month, 1 computer |
| Essential | USD 9/month | 5,000 prints/month, 3 computers |
| Standard | USD 29/month | 25,000 prints/month, 5 computers |
| Premium | USD 99/month | 200,000 prints/month, unlimited computers |

Annual variants and per-1,000-print overages are offered. The public Integrator
plans observed were USD 60/month for 100,000 prints and 20 subaccounts, and USD
500/month for 500,000 prints and 200 subaccounts, both with overage charges and
unlimited computers. Private Standalone Server pricing is by contact.

The internal business case must compare this relatively modest service price
with engineering, signing, device-lab, maintenance, security response, and
on-call costs. Open source/SaaS may still be strategically worthwhile, but
internal fee avoidance alone may not fund full public parity.

## Operational behavior worth matching or improving

- Client auto-detection after an OS printer is added.
- Service/daemon and interactive installation modes.
- Local logs and a workflow for sending a diagnostic bundle.
- Unattended Windows installation.
- Multiple rendering backends in the proprietary client.
- Connection indicators and local administration.
- Stable API error and request IDs.
- Headless operation.
- URI printing without routing content through the control plane.

Known documented trouble areas should become explicit tests:

- duplicate clients on the same machine can duplicate output;
- Windows service identity changes defaults and network-printer access;
- PDF rotation, scaling, and font embedding vary by renderer/driver;
- a delayed job is commonly a content-download delay;
- a spooler can report success even though the device never produced output;
- device and driver support for options is inconsistent.

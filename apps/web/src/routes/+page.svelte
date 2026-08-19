<script lang="ts">
  import GlobalPrintNetwork from '$lib/components/marketing/GlobalPrintNetwork.svelte';
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import PricingCards from '$lib/components/marketing/PricingCards.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import type { CloudPricingCatalog } from '$lib/marketing/types';

  let { data }: { data: { pricing: CloudPricingCatalog } } = $props();
</script>

<Seo
  title="Piqae — Cloud printing infrastructure for developers"
  description="Add reliable network printing to your product with one straightforward API, Piqae Cloud, and a lightweight local agent that keeps the full capability of every printer."
  path="/"
  structuredData={[
    {
      '@context': 'https://schema.org',
      '@type': 'Organization',
      name: 'Piqae',
      url: 'https://piqae.com',
      sameAs: ['https://github.com/piqae/piqae']
    },
    {
      '@context': 'https://schema.org',
      '@type': 'SoftwareApplication',
      name: 'Piqae',
      applicationCategory: 'DeveloperApplication',
      operatingSystem: 'Windows, macOS, Linux',
      license: 'https://www.apache.org/licenses/LICENSE-2.0'
    }
  ]}
/>

<MarketingShell announcement="Native agent releases are still preview-gated — see current availability">
  <section class="hero">
    <div class="m-container">
      <h1 class="m-display">Printing infrastructure, ready for your product.</h1>
      <p class="m-lede">
        Add network printing without building the network. Piqae gives your application one
        straightforward API, a managed cloud, and a lightweight agent beside every printer.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="/start?plan=free&source=home-hero">Start free</a>
        <a class="m-button" href="/docs/quickstart">Read the quickstart <span>→</span></a>
      </div>
      <p class="hero-proof">One API · Managed cloud · Full native-driver capability</p>
    </div>
  </section>

  <section class="product-stage m-container">
    <div class="dashboard-showcase">
      <div class="dashboard-capture">
        <img
          src="/images/piqae-dashboard-overview.png"
          width="1440"
          height="900"
          alt="Piqae dashboard showing node health, available printers, onboarding progress, attention items, and recent print jobs"
          fetchpriority="high"
        />
      </div>
    </div>
    <div class="truth-note">
      <span>One place to run every print workflow</span>
      <p>
        See printers, nodes, queues, and jobs across every customer and location. Piqae keeps
        each handoff clear, including the boundary between operating-system acceptance and the
        physical result.
      </p>
    </div>
  </section>

  <section class="m-section">
    <div class="m-container">
      <span class="m-eyebrow">How it works</span>
      <div class="section-heading">
        <h2 class="m-heading">Build once. Print wherever the work happens.</h2>
        <a href="/docs/quickstart">Open the quickstart <span>→</span></a>
      </div>
      <div class="steps">
        <article>
          <span>01</span>
          <h3>Connect one API</h3>
          <p>Send PDF or RAW jobs from the product and workflow your team already owns.</p>
        </article>
        <article>
          <span>02</span>
          <h3>Add each print location</h3>
          <p>A small native agent securely connects the printers at every site to Piqae Cloud.</p>
        </article>
        <article>
          <span>03</span>
          <h3>Use the full printer</h3>
          <p>Local operating-system drivers preserve media, tray, finishing, and vendor options.</p>
        </article>
      </div>
    </div>
  </section>

  <section class="global-section">
    <div class="global-frame">
      <div class="global-visual">
        <GlobalPrintNetwork />
      </div>
      <div class="m-container global-content">
        <div class="global-copy">
          <span class="m-eyebrow">Piqae Cloud</span>
          <h2 class="m-heading">Hosted to scale.<br />Local where it counts.</h2>
          <p class="m-lede">
            Run printing across customers and locations without operating the control plane
            yourself. Piqae keeps work durable from the cloud to the agent, then hands it to the
            real driver beside the printer.
          </p>
          <div class="global-points">
            <span><i></i> Managed cloud control plane</span>
            <span><i></i> Durable cloud and local queues</span>
            <span><i></i> Native capability at every site</span>
          </div>
          <a href="/docs/jobs">Read the job lifecycle guide <span>→</span></a>
        </div>
      </div>
    </div>
  </section>

  <section class="headless-section m-section">
    <div class="m-container headless-grid">
      <div class="headless-copy">
        <span class="m-eyebrow">Built for developers</span>
        <h2 class="m-heading">Add full-service printing. Keep building your product.</h2>
        <p class="m-lede">
          Piqae is the headless print layer behind your own experience. Keep your interface,
          customer relationships, and product logic while we handle the path to every printer.
        </p>
        <ul class="m-list">
          <li>A straightforward API for jobs, printers, status, and customer workspaces</li>
          <li>PDF and RAW printing without sending credentials into the browser</li>
          <li>Idempotency, queueing, retries, and agent connectivity already handled</li>
          <li>Your brand and workflow stay in front; Piqae stays behind the scenes</li>
        </ul>
        <div class="m-actions">
          <a class="m-button primary" href="/docs">Explore the API</a>
          <a class="m-button" href="/pricing#pro">See Pro pricing</a>
        </div>
      </div>
      <div class="api-stage" aria-label="Example of an application creating a print job with the Piqae API">
        <div class="api-top">
          <span><i></i><i></i><i></i></span>
          <strong>orders.ts</strong>
          <small>200 OK</small>
        </div>
        <pre><code><span class="code-muted">// Printing stays inside your workflow</span>
<span class="code-violet">const</span> job = <span class="code-violet">await</span> piqae.printJobs.create(&#123;
  printer: customer.defaultPrinter,
  content: &#123;
    type: <span class="code-green">'pdf'</span>,
    url: order.packingSlip
  &#125;,
  idempotencyKey: order.id
&#125;);</code></pre>
        <div class="api-result">
          <span><i></i> Job accepted</span>
          <strong>Customer workspace · Printer 04</strong>
        </div>
      </div>
    </div>
  </section>

  <section class="platform-section m-section">
    <div class="m-container platform-grid">
      <article class="platform-intro">
        <span class="m-eyebrow">For platforms and integrators</span>
        <h2 class="m-heading">Put printing inside the account your users already have.</h2>
        <p class="m-lede">
          Create an isolated Piqae customer account for each organisation from your backend,
          connect their printers, and keep the whole print experience inside your product.
        </p>
        <ul class="m-list">
          <li>One server-side platform credential manages your customer accounts</li>
          <li>Each customer gets separate workspaces, environments, printers, jobs, and usage</li>
          <li>Your users connect a local agent without needing a separate Piqae login</li>
        </ul>
        <div class="m-actions">
          <a class="m-button primary" href="/docs/platform-accounts">Explore platform accounts</a>
          <a class="m-button" href="/pricing">See integrator pricing</a>
        </div>
      </article>

      <article class="security-card">
        <span class="m-eyebrow">Protected at every handoff</span>
        <h2>Clear encryption boundaries, from cloud to printer.</h2>
        <div class="security-levels">
          <div>
            <strong>TLS in transit</strong>
            <p>Remote API, upload, and agent traffic travels over authenticated TLS.</p>
          </div>
          <div>
            <strong>Encrypted at rest</strong>
            <p>Hosted document content uses object-level encryption and short retention controls.</p>
          </div>
          <div>
            <strong>AES-256-GCM payload encryption</strong>
            <p>Preview confidential printing encrypts each job for its permitted nodes before upload.</p>
          </div>
        </div>
        <p class="security-note">
          Confidential printing is a Preview feature, not yet an independently audited end-to-end
          encryption claim. The destination operating system and printer must receive plaintext to print.
        </p>
        <a href="/security">See the security model <span>→</span></a>
      </article>
    </div>
  </section>

  <section class="decisions-section m-section">
    <div class="m-container decisions-grid">
      <div>
        <span class="m-eyebrow">Less infrastructure to operate</span>
        <h2 class="m-heading">Run the workflow, not the printing plumbing.</h2>
        <p class="m-lede">
          Piqae Cloud handles accounts, queues, agent connectivity, and status in one service.
          Your team gets the visibility to resolve problems without owning another distributed
          system.
        </p>
        <a class="m-button dark" href="/pricing">See Piqae Cloud pricing</a>
      </div>
      <div class="status-board" aria-label="Example print job status timeline">
        <div class="status-board-top">
          <div><span>Print job</span><strong>job_01J9…K4P</strong></div>
          <span class="status-chip">OS accepted</span>
        </div>
        <div class="status-document">
          <span>PDF</span>
          <div><strong>Packing slip</strong><small>1 page · Printer 04</small></div>
          <i>···</i>
        </div>
        <ol>
          <li class="complete"><i>✓</i><div><strong>Job received</strong><small>Your application · 09:41:03</small></div></li>
          <li class="complete"><i>✓</i><div><strong>Agent accepted</strong><small>Local print station · 09:41:04</small></div></li>
          <li class="complete"><i>✓</i><div><strong>Driver settings applied</strong><small>A6 label · Tray 2 · 09:41:05</small></div></li>
          <li class="current"><i></i><div><strong>OS spooler accepted</strong><small>Local handoff complete · 09:41:06</small></div></li>
        </ol>
        <p><strong>What this means:</strong> the operating system accepted the job. Physical output is not independently confirmed.</p>
      </div>
    </div>
  </section>

  <section class="advantages-section m-section">
    <div class="m-container">
      <div class="advantages-intro">
        <span class="m-eyebrow">Why Piqae</span>
        <h2 class="m-heading">A shorter path from an idea to printed output.</h2>
      </div>
      <div class="advantages-grid">
        <article>
          <span>01</span>
          <h3>Developer-forward from day one</h3>
          <p>Clear resources, useful SDK patterns, webhooks, and a dashboard that speaks the same language as the API.</p>
        </article>
        <article>
          <span>02</span>
          <h3>Dependable from cloud to edge</h3>
          <p>Durable queues and explicit recovery keep work moving through brief network and site interruptions.</p>
        </article>
        <article>
          <span>03</span>
          <h3>Real drivers. Real print capability.</h3>
          <p>Use the installed operating-system driver for specialist media, trays, finishing, and production hardware.</p>
        </article>
        <article>
          <span>04</span>
          <h3>Cloud economics that stay clear</h3>
          <p>Start free, move to straightforward usage allowances, and compare the cost against your current print volume.</p>
        </article>
      </div>
    </div>
  </section>

  <section class="switch-section">
    <div class="m-container switch-grid">
      <div>
        <span class="m-eyebrow">Switch with confidence</span>
        <h2 class="m-heading">A developer-first alternative to PrintNode.</h2>
        <p class="m-lede">
          Compare the API model, printer capability, and documented pricing against the way you
          print today. Then move one customer or location at a time with a clear rollback path.
        </p>
        <div class="m-actions">
          <a class="m-button primary" href="/compare/printnode">Compare Piqae and PrintNode</a>
          <a class="m-button" href="/tools/printnode-cost-calculator">Estimate your cost</a>
        </div>
      </div>
      <div class="migration-card">
        <div class="migration-top"><span>Migration run</span><strong>Canary workspace</strong></div>
        <ol>
          <li class="done"><span>1</span> Inventory current endpoints</li>
          <li class="done"><span>2</span> Map printers and credentials</li>
          <li class="active"><span>3</span> Route a controlled job set</li>
          <li><span>4</span> Observe status differences</li>
          <li><span>5</span> Expand or roll back</li>
        </ol>
      </div>
    </div>
  </section>

  <section class="pricing-section">
    <div class="m-container">
      <div class="pricing-intro">
        <span class="m-eyebrow">Piqae Cloud pricing</span>
        <h2 class="m-heading">Start small. Pay only for jobs reported complete.</h2>
        <p>Failed, blocked, cancelled, expired, and delivery-uncertain jobs do not count. Start free, grow with Pro, or tailor an Enterprise rollout.</p>
      </div>
      <PricingCards homepage plans={data.pricing.plans} />
      <p class="pricing-foot">
        Displayed prices are USD excluding tax. Stripe confirms the final charge at checkout.
        Public sale remains gated by cost and release readiness.
      </p>
    </div>
  </section>

  <section class="open-source-section">
    <div class="m-container open-source-grid">
      <span class="m-eyebrow">Open source when you need it</span>
      <div>
        <h2>Piqae Cloud is the easy path. The platform is still yours to inspect and run.</h2>
        <p>
          The complete printing stack is available under Apache-2.0 for teams with specific
          infrastructure, residency, or local-only requirements.
        </p>
      </div>
      <a href="/open-source">Explore open source <span>→</span></a>
    </div>
  </section>

</MarketingShell>

<style>
  .hero {
    position: relative;
    overflow: hidden;
    padding: clamp(90px, 12vw, 170px) 0 86px;
    text-align: center;
  }
  .hero::after {
    position: absolute;
    z-index: 0;
    top: -200px;
    right: -180px;
    width: 600px;
    height: 600px;
    background: radial-gradient(circle, rgb(0 106 255 / 0.12), transparent 66%);
    content: '';
    pointer-events: none;
  }
  .hero .m-container {
    position: relative;
    z-index: 1;
    display: grid;
    justify-items: center;
  }
  .hero .m-display { max-width: 1050px; margin-top: 28px; }
  .hero .m-lede { max-width: 780px; }
  .hero .m-actions { justify-content: center; }
  .hero-proof {
    margin: 22px 0 0;
    color: var(--m-faint);
    font-size: 12px;
    font-weight: 570;
    letter-spacing: 0.01em;
  }
  .product-stage {
    position: relative;
    padding-bottom: clamp(70px, 10vw, 130px);
  }
  .dashboard-showcase {
    position: relative;
    overflow: hidden;
    padding: 0;
    border: 1px solid rgb(8 9 12 / .72);
    border-radius: 18px;
    background: #08090c;
    box-shadow:
      0 48px 120px rgb(20 22 30 / .24),
      0 12px 38px rgb(20 22 30 / .12),
      inset 0 0 0 1px rgb(255 255 255 / .05);
  }
  .dashboard-showcase::before {
    position: absolute;
    z-index: -1;
    inset: 18% 6% -12%;
    border-radius: 50%;
    background: rgb(0 106 255 / .16);
    filter: blur(70px);
    content: '';
  }
  .dashboard-capture {
    position: relative;
    aspect-ratio: 1440 / 900;
    overflow: hidden;
    border: 0;
    border-radius: 17px;
    background: #08090c;
    box-shadow: none;
  }
  .dashboard-capture img {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: auto;
    display: block;
  }
  .truth-note {
    max-width: 770px;
    display: grid;
    grid-template-columns: 160px 1fr;
    gap: 22px;
    margin: 18px auto 0;
    color: var(--m-muted);
    font-size: 13px;
  }
  .truth-note span { color: var(--m-ink); font-weight: 650; }
  .truth-note p { margin: 0; }
  .section-heading {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 28px;
    margin-bottom: 48px;
  }
  .section-heading h2 { max-width: 720px; }
  .section-heading a {
    flex: none;
    color: var(--m-violet-dark);
    font-size: 13px;
    font-weight: 650;
  }
  .steps {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    border-top: 1px solid var(--m-border);
  }
  .steps article { padding: 27px 40px 0 0; }
  .steps span { color: var(--m-violet-dark); font: 11px var(--font-mono); }
  .steps h3 { margin: 25px 0 8px; font-size: 19px; letter-spacing: -0.03em; }
  .steps p { margin: 0; color: var(--m-muted); }
  .global-section {
    overflow: hidden;
    padding: 0;
    background: #020b16;
  }
  .global-frame {
    position: relative;
    min-height: 920px;
    overflow: hidden;
    background:
      radial-gradient(circle at 23% 52%, rgb(0 106 255 / .19), transparent 39%),
      radial-gradient(circle at 84% 42%, rgb(29 119 255 / .08), transparent 34%),
      #020b16;
  }
  .global-frame::after {
    position: absolute;
    z-index: 1;
    inset: 0;
    background: linear-gradient(90deg, transparent 42%, rgb(2 11 22 / .18) 55%, rgb(2 11 22 / .78) 100%);
    content: '';
    pointer-events: none;
  }
  .global-visual {
    position: absolute;
    z-index: 0;
    inset: 0;
  }
  .global-content {
    position: relative;
    z-index: 2;
    min-height: 920px;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    pointer-events: none;
  }
  .global-copy {
    width: min(43%, 530px);
    padding: 72px 0;
  }
  .global-copy .m-eyebrow { color: #71adff; }
  .global-copy .m-heading {
    max-width: 510px;
    margin-top: 18px;
    color: white;
    font-size: clamp(48px, 5.8vw, 78px);
  }
  .global-copy .m-lede {
    max-width: 500px;
    margin: 28px 0 0;
    color: #9eacbd;
    font-size: clamp(17px, 1.7vw, 21px);
  }
  .global-points {
    display: grid;
    gap: 13px;
    padding: 30px 0;
    margin-top: 30px;
    border-top: 1px solid rgb(255 255 255 / .12);
    border-bottom: 1px solid rgb(255 255 255 / .12);
    color: #d6e0ed;
    font-size: 12px;
  }
  .global-points span {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .global-points i {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #5ca3ff;
    box-shadow: 0 0 12px rgb(92 163 255 / .72);
  }
  .global-copy > a {
    display: inline-flex;
    gap: 10px;
    margin-top: 28px;
    color: white;
    font-size: 13px;
    font-weight: 680;
    pointer-events: auto;
  }
  .headless-section {
    background:
      radial-gradient(circle at 86% 50%, rgb(0 106 255 / 0.08), transparent 31%),
      #eeece6;
  }
  .headless-grid {
    display: grid;
    grid-template-columns: 0.9fr 1.1fr;
    align-items: center;
    gap: clamp(45px, 9vw, 125px);
  }
  .headless-copy .m-lede { font-size: 18px; }
  .headless-copy .m-list { max-width: 590px; }
  .api-stage {
    overflow: hidden;
    border: 1px solid rgb(255 255 255 / 0.08);
    border-radius: 22px;
    background:
      radial-gradient(circle at 80% 0%, rgb(0 106 255 / 0.18), transparent 36%),
      #0b0b0f;
    color: white;
    box-shadow: 0 32px 75px rgb(24 20 36 / 0.22);
    transform: rotate(1.2deg);
  }
  .api-top,
  .api-result {
    min-height: 48px;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 0 18px;
    border-bottom: 1px solid var(--m-border-light);
    color: #777681;
    font: 10px var(--font-mono);
  }
  .api-top > span { display: flex; gap: 5px; }
  .api-top i {
    width: 7px;
    height: 7px;
    display: block;
    border-radius: 50%;
    background: #34343d;
  }
  .api-top strong { color: #aaa8b1; font-weight: 500; }
  .api-top small { justify-self: end; color: var(--m-green); font: inherit; }
  .api-stage pre {
    min-height: 340px;
    display: flex;
    align-items: center;
    overflow-x: auto;
    padding: clamp(25px, 5vw, 48px);
    margin: 0;
    color: #d7d5dc;
    font: 13px/1.9 var(--font-mono);
  }
  .code-muted { color: #666570; }
  .code-violet { color: #79b3ff; }
  .code-green { color: #6dd5a5; }
  .api-result {
    grid-template-columns: 1fr auto;
    border-top: 1px solid var(--m-border-light);
    border-bottom: 0;
  }
  .api-result span { color: #aaa8b1; }
  .api-result span i {
    width: 6px;
    height: 6px;
    display: inline-block;
    margin-right: 7px;
    border-radius: 50%;
    background: var(--m-green);
    box-shadow: 0 0 10px rgb(62 187 129 / 0.6);
  }
  .api-result strong { color: #777681; font-weight: 500; }
  .platform-section { background: white; }
  .platform-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    align-items: stretch;
    gap: clamp(40px, 7vw, 96px);
  }
  .platform-intro .m-lede { max-width: 610px; font-size: 18px; }
  .platform-intro .m-list { max-width: 610px; }
  .security-card {
    padding: clamp(30px, 5vw, 56px);
    border-radius: 24px;
    background:
      radial-gradient(circle at 100% 0%, rgb(0 106 255 / .24), transparent 40%),
      #07111f;
    color: white;
  }
  .security-card .m-eyebrow { color: #71adff; }
  .security-card h2 { max-width: 560px; margin: 18px 0 34px; font-size: clamp(30px, 3.4vw, 46px); }
  .security-levels { display: grid; border-top: 1px solid rgb(255 255 255 / .14); }
  .security-levels > div { padding: 20px 0; border-bottom: 1px solid rgb(255 255 255 / .14); }
  .security-levels strong { font-size: 14px; }
  .security-levels p,
  .security-note { margin: 5px 0 0; color: #9eacbd; font-size: 12px; }
  .security-note { margin-top: 22px; }
  .security-card > a { display: inline-flex; gap: 10px; margin-top: 24px; color: white; font-size: 13px; font-weight: 680; }
  .decisions-section { background: #c9e0ff; }
  .decisions-grid {
    display: grid;
    grid-template-columns: .85fr 1.15fr;
    align-items: center;
    gap: clamp(50px, 9vw, 125px);
  }
  .decisions-grid .m-lede { font-size: 18px; }
  .decisions-grid .m-button { margin-top: 28px; }
  .status-board {
    overflow: hidden;
    padding: 15px;
    border-radius: 24px;
    background: white;
    box-shadow: 0 35px 90px rgb(20 65 120 / .18);
  }
  .status-board-top,
  .status-document {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 15px;
  }
  .status-board-top div,
  .status-document div { display: grid; }
  .status-board-top div > span,
  .status-board-top div > strong,
  .status-document small { color: var(--m-faint); font: 10px var(--font-mono); }
  .status-board-top div > strong { margin-top: 4px; color: var(--m-ink); font-weight: 500; }
  .status-chip {
    padding: 5px 9px;
    border-radius: 99px;
    background: #e5f7ef;
    color: #087448;
    font-size: 10px;
    font-weight: 680;
  }
  .status-document {
    justify-content: flex-start;
    border: 1px solid var(--m-border);
    border-radius: 14px;
    background: #f7f7f5;
  }
  .status-document > span {
    width: 42px;
    height: 50px;
    display: grid;
    place-items: center;
    border-radius: 7px;
    background: #111;
    color: white;
    font: 9px var(--font-mono);
  }
  .status-document div strong { font-size: 12px; }
  .status-document i { margin-left: auto; color: var(--m-faint); font-style: normal; }
  .status-board ol { display: grid; padding: 12px 15px; margin: 0; list-style: none; }
  .status-board li {
    position: relative;
    display: grid;
    grid-template-columns: 26px 1fr;
    gap: 12px;
    min-height: 63px;
  }
  .status-board li:not(:last-child)::before {
    position: absolute;
    top: 23px;
    left: 11px;
    bottom: 0;
    width: 1px;
    background: #d8d8d5;
    content: '';
  }
  .status-board li > i {
    width: 23px;
    height: 23px;
    display: grid;
    place-items: center;
    z-index: 1;
    border-radius: 50%;
    background: #111;
    color: white;
    font: 9px var(--font-mono);
  }
  .status-board li.current > i {
    border: 6px solid #d6e8ff;
    background: #006aff;
  }
  .status-board li div { display: grid; align-content: start; }
  .status-board li strong { font-size: 12px; }
  .status-board li small { color: var(--m-faint); font: 9px var(--font-mono); }
  .status-board > p {
    padding: 14px;
    margin: 0;
    border-radius: 11px;
    background: #f0f6ff;
    color: var(--m-muted);
    font-size: 10px;
  }
  .advantages-section {
    background: white;
  }
  .advantages-intro {
    max-width: 830px;
    margin-bottom: clamp(48px, 7vw, 80px);
  }
  .advantages-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    border-top: 1px solid var(--m-border);
  }
  .advantages-grid article {
    min-height: 330px;
    display: flex;
    flex-direction: column;
    padding: 28px 28px 8px 0;
  }
  .advantages-grid article + article {
    padding-left: 28px;
    border-left: 1px solid var(--m-border);
  }
  .advantages-grid article > span {
    color: var(--m-violet-dark);
    font: 600 11px var(--font-mono);
  }
  .advantages-grid h3 {
    margin: auto 0 16px;
    font-size: clamp(21px, 2.15vw, 29px);
    font-weight: 560;
    letter-spacing: -.035em;
    line-height: 1.08;
  }
  .advantages-grid p {
    min-height: 112px;
    margin: 0;
    color: var(--m-muted);
  }
  .switch-section { padding: clamp(72px, 10vw, 130px) 0; background: #ded9ce; }
  .switch-grid { display: grid; grid-template-columns: 1.15fr .85fr; align-items: center; gap: clamp(40px, 8vw, 110px); }
  .migration-card {
    padding: 14px;
    border: 1px solid rgb(255 255 255 / 0.08);
    border-radius: 18px;
    background: var(--m-dark);
    color: white;
    box-shadow: 0 28px 70px rgb(23 22 27 / .2);
  }
  .migration-top {
    display: flex;
    justify-content: space-between;
    padding: 8px 8px 17px;
    border-bottom: 1px solid var(--m-border-light);
    color: #777681;
    font: 10px var(--font-mono);
  }
  .migration-top strong { color: #c8c6ce; font-weight: 500; }
  .migration-card ol { display: grid; gap: 7px; padding: 18px 0 0; margin: 0; list-style: none; }
  .migration-card li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px;
    border: 1px solid transparent;
    border-radius: 9px;
    color: #6f6e78;
    font-size: 12px;
  }
  .migration-card li span {
    width: 22px; height: 22px; display: grid; place-items: center;
    border: 1px solid #363640; border-radius: 6px; font: 9px var(--font-mono);
  }
  .migration-card li.done { color: #9b99a3; }
  .migration-card li.done span { color: var(--m-green); border-color: rgb(62 187 129 / .35); }
  .migration-card li.active {
    border-color: rgb(0 106 255 / .35); background: rgb(0 106 255 / .1); color: white;
  }
  .pricing-section {
    min-height: min(900px, 100svh);
    padding: clamp(92px, 10vw, 128px) 0 clamp(92px, 10vw, 120px);
    background: #050505;
    color: white;
  }
  .pricing-intro { max-width: 850px; }
  .pricing-intro .m-eyebrow {
    margin-bottom: 30px;
    color: #8c8c8c;
    font: 600 13px/1 var(--font-mono);
    letter-spacing: .09em;
    text-transform: uppercase;
  }
  .pricing-intro .m-heading {
    color: white;
    font-size: clamp(42px, 5.2vw, 66px);
  }
  .pricing-intro p {
    max-width: 700px;
    margin: 24px 0 0;
    color: #b5b5b5;
    font-size: 17px;
  }
  .pricing-foot {
    max-width: 760px;
    margin: 58px 0 0;
    color: #858585;
    font-size: 11px;
  }
  .open-source-section {
    padding: clamp(62px, 8vw, 100px) 0;
    background: #d9eaff;
  }
  .open-source-grid {
    display: grid;
    grid-template-columns: .7fr 1.7fr auto;
    align-items: start;
    gap: clamp(28px, 5vw, 70px);
  }
  .open-source-grid .m-eyebrow {
    color: #42607f;
  }
  .open-source-grid h2 {
    max-width: 660px;
    margin: 0;
    font-family: var(--m-font-editorial);
    font-size: clamp(30px, 3.6vw, 48px);
    font-weight: 400;
    letter-spacing: -.035em;
    line-height: 1.02;
  }
  .open-source-grid p {
    max-width: 620px;
    margin: 22px 0 0;
    color: #4b5e72;
    font-size: 16px;
  }
  .open-source-grid > a {
    display: inline-flex;
    gap: 9px;
    padding-top: 5px;
    color: var(--m-violet-dark);
    font-weight: 680;
    white-space: nowrap;
  }
  @media (max-width: 900px) {
    .headless-grid,
    .platform-grid,
    .decisions-grid,
    .switch-grid,
    .open-source-grid { grid-template-columns: 1fr; }
    .advantages-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .advantages-grid article:nth-child(3) {
      padding-left: 0;
      border-left: 0;
      border-top: 1px solid var(--m-border);
    }
    .advantages-grid article:nth-child(4) {
      border-top: 1px solid var(--m-border);
    }
    .open-source-grid > a { padding-top: 0; }
    .global-frame,
    .global-content { min-height: 1050px; }
    .global-frame::after {
      background: linear-gradient(180deg, rgb(0 20 18 / .92) 0%, rgb(0 20 18 / .7) 31%, transparent 58%);
    }
    .global-content {
      align-items: flex-start;
      justify-content: flex-start;
    }
    .global-copy {
      width: min(100%, 650px);
      padding: 64px 0 0;
    }
    .global-copy .m-lede { max-width: 640px; }
    .api-stage { transform: none; }
  }
  @media (max-width: 680px) {
    .truth-note { grid-template-columns: 1fr; gap: 5px; }
    .section-heading { align-items: flex-start; flex-direction: column; }
    .steps,
    .advantages-grid { grid-template-columns: 1fr; }
    .steps { gap: 30px; }
    .advantages-grid article,
    .advantages-grid article + article,
    .advantages-grid article:nth-child(3) {
      min-height: 0;
      padding: 34px 0;
      border-top: 1px solid var(--m-border);
      border-left: 0;
    }
    .advantages-grid h3 { margin: 54px 0 14px; }
    .advantages-grid p { min-height: 0; }
    .api-stage pre { min-height: 285px; font-size: 11px; }
    .api-result { align-items: start; grid-template-columns: 1fr; gap: 4px; padding-block: 12px; }
    .dashboard-showcase { border-radius: 12px; }
    .dashboard-capture { border-radius: 11px; }
    .global-copy { padding: 48px 0 0; }
    .global-copy .m-heading { font-size: clamp(44px, 13vw, 62px); }
  }
</style>

<script lang="ts">
  import FlowDiagram from '$lib/components/marketing/FlowDiagram.svelte';
  import GlobalPrintNetwork from '$lib/components/marketing/GlobalPrintNetwork.svelte';
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import PricingCards from '$lib/components/marketing/PricingCards.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import WorkflowExplorer from '$lib/components/marketing/WorkflowExplorer.svelte';
  import type { CloudPricingCatalog } from '$lib/marketing/types';

  let { data }: { data: { pricing: CloudPricingCatalog } } = $props();
</script>

<Seo
  title="Piqae — Open printing infrastructure"
  description="Build reliable PDF and RAW printing into any product with a simple API, a lightweight local agent, and your choice of Piqae Cloud or self-hosting."
  path="/"
  structuredData={[
    {
      '@context': 'https://schema.org',
      '@type': 'Organization',
      name: 'Piqae',
      url: 'https://piqae.com',
      sameAs: ['https://github.com/C4CoffeeCo/piqae']
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
      <span class="m-pill">Open source · Cloud or self-hosted</span>
      <h1 class="m-display">Reliable printing, built into your product.</h1>
      <p class="m-lede">
        Connect your application to printers anywhere with one API and a lightweight local agent.
        Use Piqae as a complete service, run it yourself, or make it the headless print layer
        behind your own product.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="/start?plan=free&source=home-hero">Start building free</a>
        <a class="m-button" href="/how-it-works">See how it works <span>→</span></a>
      </div>
      <p class="hero-proof">PDF + RAW · Native local agent · Installed drivers stay in control</p>
    </div>
  </section>

  <section class="product-stage m-container">
    <div class="m-dark-panel">
      <FlowDiagram />
    </div>
    <div class="truth-note">
      <span>Clear status at every handoff</span>
      <p>
        See when a job is queued, on its way, or accepted by the local print system. Because
        software cannot always confirm what happened on paper, Piqae keeps that final boundary
        clear too.
      </p>
    </div>
  </section>

  <section class="m-section">
    <div class="m-container">
      <span class="m-eyebrow">How it works</span>
      <div class="section-heading">
        <h2 class="m-heading">From your app to the right printer, in three steps.</h2>
        <a href="/how-it-works">Explore the print path <span>→</span></a>
      </div>
      <div class="steps">
        <article>
          <span>01</span>
          <h3>Send the job</h3>
          <p>Your application sends PDF or RAW content to one straightforward API.</p>
        </article>
        <article>
          <span>02</span>
          <h3>Reach the right site</h3>
          <p>Piqae routes it to the enrolled agent beside the selected printer, with recovery built in.</p>
        </article>
        <article>
          <span>03</span>
          <h3>Print with local capability</h3>
          <p>The operating system applies the installed driver and reports the handoff back to your app.</p>
        </article>
      </div>
    </div>
  </section>

  <WorkflowExplorer />

  <section class="global-section">
    <div class="global-frame">
      <div class="global-visual">
        <GlobalPrintNetwork />
      </div>
      <div class="m-container global-content">
        <div class="global-copy">
          <span class="m-eyebrow">Printing without borders</span>
          <h2 class="m-heading">Global reach.<br />Local precision.</h2>
          <p class="m-lede">
            Send work across the world, then print with the full capability of the computer and
            driver beside each printer. One reliable path from your product to every site.
          </p>
          <div class="global-points">
            <span><i></i> Durable from cloud to edge</span>
            <span><i></i> Native driver capability</span>
            <span><i></i> Clear status at every handoff</span>
          </div>
          <a href="/how-it-works">See the print path <span>→</span></a>
        </div>
      </div>
    </div>
  </section>

  <section class="headless-section m-section">
    <div class="m-container headless-grid">
      <div class="headless-copy">
        <span class="m-eyebrow">Headless printing</span>
        <h2 class="m-heading">Your product. Now with printing built in.</h2>
        <p class="m-lede">
          Keep your interface, your customer relationships, and your workflow. Piqae provides the
          print infrastructure behind the scenes, from customer onboarding to local handoff.
        </p>
        <ul class="m-list">
          <li>Offer printing under your own brand and inside your own app</li>
          <li>Separate customer workspaces, agents, printers, and usage</li>
          <li>Use one API for PDF, RAW, printer discovery, and status</li>
          <li>Start managed, self-host later, or run the stack yourself from day one</li>
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
<span class="code-violet">const</span> job = <span class="code-violet">await</span> spool.printJobs.create(&#123;
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

  <section class="decisions-section m-section">
    <div class="m-container decisions-grid">
      <div>
        <span class="m-eyebrow">Clear operational insight</span>
        <h2 class="m-heading">Know what happened, fast.</h2>
        <p class="m-lede">
          Follow each job from your application to the local print system. Piqae turns queue,
          agent, and driver events into a timeline your team can understand and act on.
        </p>
        <a class="m-button dark" href="/how-it-works#status">Explore job status</a>
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

  <section class="m-section">
    <div class="m-container">
      <span class="m-eyebrow">Freedom to choose</span>
      <h2 class="m-heading">One platform. Three ways to run it.</h2>
      <div class="deployment-grid">
        <article class="m-card">
          <span class="mode">Fastest start</span>
          <h3>Piqae Cloud</h3>
          <p>We operate the service, queues, and updates so your team can focus on the product.</p>
          <a href="/pricing">See Cloud pricing →</a>
        </article>
        <article class="m-card">
          <span class="mode">Maximum control</span>
          <h3>Self-hosted</h3>
          <p>Run the complete Apache-2.0 stack in your environment, with no software licence fee.</p>
          <a href="/open-source">Explore self-hosting →</a>
        </article>
        <article class="m-card">
          <span class="mode">Entirely local</span>
          <h3>Local-only</h3>
          <p>Keep the workflow on a trusted network when cloud coordination is not needed.</p>
          <a href="/docs/local-mode">Read local-mode docs →</a>
        </article>
      </div>
    </div>
  </section>

  <section class="switch-section">
    <div class="m-container switch-grid">
      <div>
        <span class="m-eyebrow">A more open path</span>
        <h2 class="m-heading">Considering a move from PrintNode?</h2>
        <p class="m-lede">
          See how deployment, pricing, source access, and printer control compare. Then plan a
          measured rollout around the endpoints and printers you already use.
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
        <span class="m-eyebrow">Pricing</span>
        <h2 class="m-heading">One plan for every print workflow.</h2>
        <p>Start free, move to Pro when you need more volume, or run Piqae yourself.</p>
      </div>
      <PricingCards homepage plans={data.pricing.plans} />
      <p class="pricing-foot">
        Displayed prices are USD excluding tax. Stripe confirms the final charge at checkout.
        Public sale remains gated by cost and release readiness.
      </p>
    </div>
  </section>

  <section class="origin m-section">
    <div class="m-container origin-grid">
      <div class="photo-frame" role="img" aria-label="Reserved for cleared C4 Coffee workflow photography">
        <span>C4</span>
        <div>
          <strong>Real workflow photography</strong>
          <small>CMS media slot · rights clearance required</small>
        </div>
      </div>
      <div class="origin-copy">
        <span class="m-eyebrow">Why we built it</span>
        <h2 class="m-heading">Built where every label matters.</h2>
        <p>
          Piqae began at C4 Coffee, where coffee bags, shipping labels, and daily production all
          depend on software reaching the right printer at the right moment.
        </p>
        <p>
          That practical beginning still shapes the product: lightweight software, resilient
          workflows, clear status, and the freedom to choose how it runs.
        </p>
        <a class="m-button" href="/about">Read the C4 story <span>→</span></a>
      </div>
    </div>
  </section>

  <section class="final-cta">
    <div class="m-container m-dark-panel">
      <div>
        <span>Start with one workflow</span>
        <h2>Bring reliable printing into your product.</h2>
        <p>Connect the API, enroll a local agent, and keep every deployment option open.</p>
      </div>
      <div class="m-actions">
        <a class="m-button primary" href="/start?plan=free&source=home-final">Start building free</a>
        <a class="m-button" href="/docs/quickstart">Read the quickstart</a>
      </div>
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
  .product-stage { padding-bottom: clamp(70px, 10vw, 130px); }
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
  .deployment-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; margin-top: 45px; }
  .deployment-grid .m-card { min-height: 265px; display: flex; flex-direction: column; }
  .deployment-grid h3 { margin-top: 60px; font-size: 22px; }
  .deployment-grid a { margin-top: auto; padding-top: 20px; }
  .mode { color: var(--m-violet-dark); font-size: 11px; font-weight: 700; text-transform: uppercase; }
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
  .origin { background: #eeece6; }
  .origin-grid { display: grid; grid-template-columns: 1fr 1fr; align-items: center; gap: clamp(42px, 8vw, 110px); }
  .photo-frame {
    min-height: 540px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: 32px;
    border-radius: 22px;
    background:
      linear-gradient(160deg, rgb(25 23 20 / .05), rgb(25 23 20 / .78)),
      repeating-linear-gradient(45deg, #c7bba5, #c7bba5 14px, #bcaf98 14px, #bcaf98 28px);
    color: white;
    box-shadow: inset 0 0 0 1px rgb(23 22 27 / .08);
  }
  .photo-frame > span { font-size: 80px; font-weight: 750; letter-spacing: -.08em; }
  .photo-frame div { display: grid; }
  .photo-frame small { color: rgb(255 255 255 / .68); }
  .origin-copy p { color: var(--m-muted); font-size: 17px; }
  .origin-copy .m-button { margin-top: 18px; }
  .final-cta { padding: 80px 0; }
  .final-cta .m-dark-panel {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 40px;
    padding: clamp(35px, 6vw, 72px);
  }
  .final-cta span { color: #898792; font-size: 12px; }
  .final-cta h2 { max-width: 640px; margin: 10px 0; font-size: clamp(34px, 5vw, 61px); letter-spacing: -.055em; }
  .final-cta p { max-width: 600px; margin: 0; }
  .final-cta .m-actions { flex: none; }
  .final-cta .m-button:not(.primary) { border-color: var(--m-border-light); background: transparent; color: white; }
  @media (max-width: 900px) {
    .headless-grid,
    .decisions-grid,
    .switch-grid,
    .origin-grid { grid-template-columns: 1fr; }
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
    .final-cta .m-dark-panel { align-items: flex-start; flex-direction: column; }
  }
  @media (max-width: 680px) {
    .truth-note { grid-template-columns: 1fr; gap: 5px; }
    .section-heading { align-items: flex-start; flex-direction: column; }
    .steps, .deployment-grid { grid-template-columns: 1fr; }
    .steps { gap: 30px; }
    .api-stage pre { min-height: 285px; font-size: 11px; }
    .api-result { align-items: start; grid-template-columns: 1fr; gap: 4px; padding-block: 12px; }
    .photo-frame { min-height: 420px; }
    .global-copy { padding: 48px 0 0; }
    .global-copy .m-heading { font-size: clamp(44px, 13vw, 62px); }
  }
</style>

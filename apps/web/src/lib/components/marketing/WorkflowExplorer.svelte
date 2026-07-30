<script lang="ts">
  const workflows = [
    {
      label: 'Build printing into your app',
      body: 'Give customers complete network printing inside your own product with one headless API and a local agent that stays out of the way.',
      href: '/docs/quickstart',
      visual: 'embedded',
      stageLabel: 'Inside your product'
    },
    {
      label: 'Print every order',
      body: 'Send shipping labels and packing slips from your order workflow to the right station, across one site or hundreds.',
      href: '/how-it-works',
      visual: 'shipping',
      stageLabel: 'Fulfilment'
    },
    {
      label: 'Power production printing',
      body: 'Print product labels, batch documents, and operational work with the media, trays, finishing, and driver options each site already uses.',
      href: '/how-it-works',
      visual: 'production',
      stageLabel: 'Production'
    },
    {
      label: 'Automate service printing',
      body: 'Route receipts, tickets, and work documents to the people who need them without adding another manual step.',
      href: '/how-it-works',
      visual: 'service',
      stageLabel: 'Service'
    },
    {
      label: 'Run every customer location',
      body: 'Give your product one dependable print layer across stores, warehouses, offices, and customer sites while local drivers keep the final say.',
      href: '/how-it-works',
      visual: 'locations',
      stageLabel: 'Multi-site operations'
    }
  ] as const;

  let activeWorkflow = $state(0);
  const selected = $derived(workflows[activeWorkflow] ?? workflows[0]);
</script>

<section class="workflow-explorer" aria-labelledby="workflow-explorer-title">
  <div class="explorer-grid">
    <div class="sidebar-shell">
      <div class="sidebar">
        <header>
          <span class="m-eyebrow">One platform, every workflow</span>
          <h2 id="workflow-explorer-title">See your whole print operation click into place.</h2>
        </header>

        <div class="workflow-accordion">
          {#each workflows as workflow, index}
            <article class:active={activeWorkflow === index}>
              <h3>
                <button
                  type="button"
                  aria-expanded={activeWorkflow === index}
                  aria-controls={`workflow-panel-${index}`}
                  onclick={() => (activeWorkflow = index)}
                >
                  {workflow.label}
                  <span aria-hidden="true">+</span>
                </button>
              </h3>
              <div
                id={`workflow-panel-${index}`}
                class="workflow-detail"
                hidden={activeWorkflow !== index}
              >
                <p>{workflow.body}</p>
                <a href={workflow.href}>Learn more <span aria-hidden="true">→</span></a>
              </div>
            </article>
          {/each}
        </div>
      </div>
    </div>

    <div
      class={`workflow-stage ${selected.visual}`}
      role="img"
      aria-label={`${selected.stageLabel} printing with Piqae`}
    >
      {#if selected.visual === 'embedded' || selected.visual === 'locations'}
        <picture class="stage-media dashboard-media" aria-hidden="true">
          <img
            src="/images/piqae-dashboard-overview.png"
            width="1440"
            height="900"
            loading="lazy"
            alt=""
          />
        </picture>
      {:else if selected.visual === 'shipping'}
        <picture class="stage-media" aria-hidden="true">
          <source srcset="/images/piqae-fulfilment.avif" type="image/avif" />
          <img
            src="/images/piqae-fulfilment.jpg"
            width="1448"
            height="1086"
            loading="lazy"
            alt=""
          />
        </picture>
      {:else if selected.visual === 'production'}
        <picture class="stage-media" aria-hidden="true">
          <source srcset="/images/piqae-industrial-print.avif" type="image/avif" />
          <img
            src="/images/piqae-industrial-print.jpg"
            width="1448"
            height="1086"
            loading="lazy"
            alt=""
          />
        </picture>
      {:else}
        <picture class="stage-media" aria-hidden="true">
          <source srcset="/images/piqae-label-production.avif" type="image/avif" />
          <img
            src="/images/piqae-label-production.jpg"
            width="1448"
            height="1086"
            loading="lazy"
            alt=""
          />
        </picture>
      {/if}

      {#if selected.visual === 'embedded'}
        <div class="product-window">
          <div class="window-bar">
            <span><i></i><i></i><i></i></span>
            <small>YOUR APP</small>
            <b>•••</b>
          </div>
          <div class="app-shell">
            <aside><strong>ORBIT</strong><span></span><span></span><span></span><span></span></aside>
            <div class="app-content">
              <small>ORDER #1048</small>
              <h4>Ready to dispatch</h4>
              <div class="order-lines"><span></span><span></span><span></span></div>
              <div class="print-drawer">
                <span>Print shipping documents</span>
                <strong>Warehouse · Station 03</strong>
                <span class="mock-print-button">Print now</span>
                <small><i></i> Powered by Piqae</small>
              </div>
            </div>
          </div>
        </div>
      {:else if selected.visual === 'shipping'}
        <div class="warehouse-scene">
          <div class="shipping-sheet">
            <small>PIQAE PRIORITY</small>
            <div class="barcode"></div>
            <strong>AKL 04</strong>
            <span>Order SP-1048</span>
            <span>Station 03 · Label ready</span>
          </div>
          <div class="route-chip"><i></i> Routed to the right station</div>
        </div>
      {:else if selected.visual === 'production'}
        <div class="production-scene">
          <div class="product-label">
            <small>NATIVE PRINT PROFILE</small>
            <strong>Packaging line 03</strong>
            <span>102 × 152 mm · Cutter enabled</span>
            <i>Driver ready</i>
          </div>
          <div class="driver-chip"><i></i> Local media profile applied</div>
        </div>
      {:else if selected.visual === 'service'}
        <div class="service-scene">
          <div class="service-board">
            <small>LIVE ORDERS</small>
            <div><span>017</span><strong>In progress</strong></div>
            <div class="selected"><span>018</span><strong>Ready to print</strong></div>
            <div><span>019</span><strong>Queued</strong></div>
          </div>
          <div class="service-ticket">
            <small>ORDER READY</small>
            <strong>#018</strong>
            <span>Pickup counter</span>
            <i></i>
            <b>Printed at Station 02</b>
          </div>
        </div>
      {:else}
        <div class="locations-scene">
          <div class="location-map" aria-hidden="true">
            <span class="route route-one"></span>
            <span class="route route-two"></span>
            <i class="point point-one"></i>
            <i class="point point-two"></i>
            <i class="point point-three"></i>
          </div>
          <div class="locations-panel">
            <small>PRINT LOCATIONS</small>
            <div><i></i><span><strong>Central warehouse</strong><b>8 printers ready</b></span></div>
            <div><i></i><span><strong>City store</strong><b>3 printers ready</b></span></div>
            <div><i></i><span><strong>Customer site 14</strong><b>2 printers ready</b></span></div>
            <footer><span>13 printers</span><strong>One print layer</strong></footer>
          </div>
        </div>
      {/if}
    </div>
  </div>
</section>

<style>
  .workflow-explorer {
    padding: 20px 0;
    background: #fff;
    color: #0a0a0a;
  }
  .explorer-grid {
    min-height: 1040px;
    display: grid;
    grid-template-columns: minmax(430px, 42.45%) minmax(0, 57.55%);
  }
  .sidebar-shell {
    display: flex;
    justify-content: flex-end;
    padding: 198px clamp(42px, 5.2vw, 76px) 90px max(30px, calc((100vw - 1160px) / 2));
  }
  .sidebar {
    width: min(100%, 390px);
  }
  header {
    margin-bottom: 60px;
  }
  header .m-eyebrow {
    color: #777;
    font-family: var(--font-mono);
  }
  h2 {
    max-width: 340px;
    margin: 20px 0 0;
    font-family: var(--m-font-editorial);
    font-size: clamp(40px, 3.2vw, 48px);
    font-weight: 400;
    letter-spacing: -.035em;
    line-height: 1.06;
  }
  .workflow-accordion {
    border-bottom: 1px solid #ccc;
  }
  article {
    border-top: 1px solid #ccc;
  }
  article:first-child {
    border-top: 0;
  }
  h3 {
    margin: 0;
  }
  h3 button {
    width: 100%;
    min-height: 65px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    padding: 10px 0;
    border: 0;
    background: transparent;
    color: #777;
    cursor: pointer;
    font: 500 18px/1.3 var(--m-font-display);
    letter-spacing: -.025em;
    text-align: left;
    transition: color 180ms ease;
  }
  h3 button:hover,
  h3 button:focus-visible,
  article.active h3 button {
    color: #080808;
  }
  h3 button > span {
    flex: none;
    font-family: var(--m-font-display);
    font-size: 21px;
    font-weight: 350;
    transition: transform 200ms ease;
  }
  article.active h3 button > span {
    transform: rotate(45deg);
  }
  .workflow-detail {
    padding: 0 30px 30px 0;
  }
  .workflow-detail[hidden] {
    display: none;
  }
  .workflow-detail p {
    margin: -2px 0 22px;
    color: #4d4d4d;
    font-size: 16px;
    line-height: 1.5;
  }
  .workflow-detail a {
    color: #080808;
    font-size: 15px;
    font-weight: 600;
  }
  .workflow-detail a span {
    margin-left: 4px;
  }
  .workflow-stage {
    position: relative;
    min-width: 0;
    min-height: 1040px;
    display: grid;
    place-items: center;
    overflow: hidden;
    isolation: isolate;
    transition: background-color 220ms ease;
  }
  .workflow-stage::before {
    position: absolute;
    z-index: 1;
    inset: 0;
    background:
      linear-gradient(180deg, rgb(8 13 20 / .06), rgb(8 13 20 / .32)),
      linear-gradient(90deg, rgb(0 55 135 / .08), transparent 58%);
    content: '';
    pointer-events: none;
  }
  .workflow-stage::after {
    position: absolute;
    z-index: 1;
    inset: 0;
    background-image: radial-gradient(rgb(255 255 255 / .18) .65px, transparent .65px);
    background-size: 4px 4px;
    content: '';
    opacity: .16;
    pointer-events: none;
  }
  .workflow-stage > :not(.stage-media) {
    position: relative;
    z-index: 2;
  }
  .stage-media {
    position: absolute;
    z-index: 0;
    inset: 0;
    display: block;
  }
  .stage-media img {
    width: 100%;
    height: 100%;
    display: block;
    object-fit: cover;
    transform: scale(1.015);
  }
  .dashboard-media img {
    filter: saturate(.72) contrast(.86) brightness(.72);
    object-position: 38% center;
    transform: scale(1.18);
  }
  .workflow-stage.embedded { background: #9bc8ff; }
  .workflow-stage.embedded::before {
    background:
      linear-gradient(135deg, rgb(104 173 255 / .7), rgb(130 189 255 / .32)),
      rgb(0 106 255 / .18);
  }
  .workflow-stage.shipping { background: #af815f; }
  .workflow-stage.production { background: #746f65; }
  .workflow-stage.service { background: #7fae9c; }
  .workflow-stage.locations { background: #7d88c2; }
  .product-window,
  .service-board,
  .locations-panel {
    background: #fff;
    box-shadow: 0 45px 100px rgb(23 31 43 / .24);
  }
  .product-window {
    width: min(78%, 580px);
    overflow: hidden;
    border: 1px solid rgb(255 255 255 / .58);
    border-radius: 22px;
    box-shadow: 0 45px 100px rgb(23 31 43 / .38);
    transform: rotate(-1.5deg);
  }
  .window-bar {
    min-height: 52px;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 0 18px;
    border-bottom: 1px solid #e5e5e5;
  }
  .window-bar > span {
    display: flex;
    gap: 5px;
  }
  .window-bar i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #d7d7d7;
  }
  .window-bar small {
    color: #777;
    font: 600 9px var(--font-mono);
    letter-spacing: .12em;
  }
  .window-bar b {
    justify-self: end;
    color: #aaa;
    letter-spacing: 2px;
  }
  .app-shell {
    min-height: 560px;
    display: grid;
    grid-template-columns: 110px 1fr;
  }
  .app-shell aside {
    display: grid;
    align-content: start;
    gap: 20px;
    padding: 26px 18px;
    background: #101317;
    color: white;
  }
  .app-shell aside strong {
    margin-bottom: 24px;
    font-size: 13px;
    letter-spacing: .14em;
  }
  .app-shell aside span {
    height: 5px;
    border-radius: 9px;
    background: #34383e;
  }
  .app-content {
    position: relative;
    padding: 45px 34px;
    background: #f5f5f2;
  }
  .app-content > small {
    color: #777;
    font: 10px var(--font-mono);
  }
  .app-content h4 {
    margin: 10px 0 30px;
    font-size: 30px;
    letter-spacing: -.05em;
  }
  .order-lines {
    display: grid;
    gap: 12px;
  }
  .order-lines span {
    height: 10px;
    border-radius: 10px;
    background: #deded9;
  }
  .order-lines span:nth-child(2) { width: 82%; }
  .order-lines span:nth-child(3) { width: 60%; }
  .print-drawer {
    position: absolute;
    right: 24px;
    bottom: 24px;
    left: 24px;
    display: grid;
    padding: 24px;
    border: 1px solid #e0e0dc;
    border-radius: 14px;
    background: white;
    box-shadow: 0 20px 40px rgb(0 0 0 / .1);
  }
  .print-drawer > span {
    font-size: 12px;
    font-weight: 650;
  }
  .print-drawer > strong {
    margin-top: 7px;
    font-size: 19px;
  }
  .mock-print-button {
    min-height: 42px;
    display: grid;
    place-items: center;
    margin-top: 22px;
    border-radius: 8px;
    background: #006aff;
    color: white;
    font-size: 13px;
    font-weight: 650;
  }
  .print-drawer small {
    margin-top: 12px;
    color: #8b8b8b;
    font: 9px var(--font-mono);
    text-align: center;
  }
  .print-drawer small i,
  .driver-chip i,
  .route-chip i {
    width: 6px;
    height: 6px;
    display: inline-block;
    margin-right: 5px;
    border-radius: 50%;
    background: #006aff;
  }
  .warehouse-scene,
  .production-scene,
  .service-scene,
  .locations-scene {
    position: relative;
    width: 100%;
    height: 100%;
  }
  .shipping-sheet {
    position: absolute;
    top: 26%;
    left: 19%;
    width: min(48%, 360px);
    min-height: 450px;
    display: flex;
    flex-direction: column;
    padding: 38px;
    border: 1px solid #111;
    background: #fff;
    box-shadow: 0 40px 80px rgb(82 50 28 / .3);
    transform: rotate(-3deg);
  }
  .shipping-sheet small {
    font: 10px var(--font-mono);
    letter-spacing: .14em;
  }
  .barcode {
    height: 100px;
    margin-top: 50px;
    background: repeating-linear-gradient(90deg, #111 0 3px, transparent 3px 7px, #111 7px 12px, transparent 12px 15px);
  }
  .shipping-sheet strong {
    margin-top: auto;
    font-size: 58px;
    letter-spacing: -.06em;
  }
  .shipping-sheet span {
    margin-top: 8px;
    font-size: 12px;
  }
  .route-chip,
  .driver-chip {
    position: absolute;
    right: 8%;
    bottom: 12%;
    padding: 14px 17px;
    border-radius: 10px;
    background: #101317;
    color: white;
    font: 10px var(--font-mono);
    box-shadow: 0 18px 45px rgb(0 0 0 / .2);
  }
  .production-scene {
    background:
      linear-gradient(90deg, transparent 0 65%, rgb(255 255 255 / .18) 65%),
      linear-gradient(0deg, rgb(82 55 24 / .16), transparent 35%);
  }
  .product-label {
    position: absolute;
    z-index: 2;
    right: 8%;
    bottom: 12%;
    width: min(55%, 420px);
    min-height: 235px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: 30px;
    border: 1px solid rgb(255 255 255 / .62);
    border-radius: 16px;
    background: #fff;
    box-shadow: 0 30px 70px rgb(28 34 42 / .28);
    text-align: left;
    transform: rotate(1deg);
  }
  .product-label small { font: 9px var(--font-mono); letter-spacing: .15em; }
  .product-label strong { max-width: 290px; margin-top: auto; font-size: 32px; letter-spacing: -.055em; line-height: 1; }
  .product-label span { margin-top: 8px; font-size: 12px; }
  .product-label i {
    margin-top: 24px;
    padding: 6px 9px;
    border-radius: 99px;
    background: #e6f7ef;
    color: #087448;
    font: 10px var(--font-mono);
  }
  .production-scene .driver-chip {
    top: 10%;
    bottom: auto;
  }
  .service-board {
    position: absolute;
    top: 21%;
    left: 10%;
    width: 52%;
    padding: 28px;
    border-radius: 18px;
    transform: rotate(-2deg);
  }
  .service-board > small,
  .locations-panel > small {
    color: #777;
    font: 600 9px var(--font-mono);
    letter-spacing: .12em;
  }
  .service-board > div {
    display: grid;
    grid-template-columns: 45px 1fr;
    align-items: center;
    gap: 17px;
    padding: 20px 4px;
    border-bottom: 1px solid #e5e5e5;
  }
  .service-board > div.selected {
    padding-inline: 14px;
    border: 0;
    border-radius: 10px;
    background: #e8f4ff;
  }
  .service-board div span {
    font: 12px var(--font-mono);
  }
  .service-board div strong {
    font-size: 15px;
  }
  .service-ticket {
    position: absolute;
    z-index: 2;
    right: 8%;
    bottom: 16%;
    width: 37%;
    min-height: 390px;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 35px 25px;
    background: #fff;
    box-shadow: 0 40px 80px rgb(23 74 51 / .24);
    text-align: center;
    transform: rotate(4deg);
  }
  .service-ticket small { font: 10px var(--font-mono); }
  .service-ticket strong { margin-top: 45px; font-size: 67px; letter-spacing: -.08em; }
  .service-ticket span { font-size: 13px; }
  .service-ticket i {
    width: 100%;
    margin-top: auto;
    border-top: 1px dashed #999;
  }
  .service-ticket b { margin-top: 22px; font: 9px var(--font-mono); }
  .location-map {
    position: absolute;
    inset: 0;
    background:
      radial-gradient(circle at 20% 25%, rgb(255 255 255 / .32), transparent 3%),
      linear-gradient(28deg, transparent 46%, rgb(255 255 255 / .2) 47% 48%, transparent 49%),
      linear-gradient(-18deg, transparent 50%, rgb(255 255 255 / .18) 51% 52%, transparent 53%);
  }
  .location-map .point {
    position: absolute;
    width: 15px;
    height: 15px;
    border: 4px solid white;
    border-radius: 50%;
    background: #006aff;
    box-shadow: 0 0 0 8px rgb(0 106 255 / .16);
  }
  .point-one { top: 21%; left: 18%; }
  .point-two { top: 49%; right: 16%; }
  .point-three { bottom: 17%; left: 28%; }
  .locations-panel {
    position: absolute;
    top: 23%;
    left: 16%;
    width: min(68%, 500px);
    padding: 30px;
    border-radius: 18px;
    transform: rotate(-1.5deg);
  }
  .locations-panel > div {
    display: grid;
    grid-template-columns: 12px 1fr;
    align-items: center;
    gap: 15px;
    padding: 20px 0;
    border-bottom: 1px solid #e5e5e5;
  }
  .locations-panel > div > i {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #37b97f;
    box-shadow: 0 0 0 4px rgb(55 185 127 / .13);
  }
  .locations-panel div span {
    display: grid;
    gap: 3px;
  }
  .locations-panel div strong {
    font-size: 14px;
  }
  .locations-panel div b {
    color: #888;
    font-size: 10px;
    font-weight: 500;
  }
  .locations-panel footer {
    display: flex;
    justify-content: space-between;
    padding-top: 25px;
    color: #777;
    font-size: 11px;
  }
  .locations-panel footer strong {
    color: #111;
  }
  @media (max-width: 900px) {
    .workflow-explorer { padding: 0; }
    .explorer-grid {
      min-height: 0;
      grid-template-columns: 1fr;
    }
    .sidebar-shell {
      justify-content: flex-start;
      padding: 88px max(24px, calc((100vw - 720px) / 2)) 70px;
    }
    .sidebar {
      width: 100%;
    }
    header {
      max-width: 620px;
    }
    h2 {
      max-width: 600px;
      font-size: clamp(43px, 7vw, 62px);
    }
    .workflow-stage {
      min-height: 720px;
    }
  }
  @media (max-width: 560px) {
    .sidebar-shell {
      padding-block: 70px 55px;
    }
    header {
      margin-bottom: 45px;
    }
    h2 {
      font-size: 43px;
    }
    h3 button {
      font-size: 17px;
    }
    .workflow-detail {
      padding-right: 0;
    }
    .workflow-stage {
      min-height: 590px;
    }
    .product-window {
      width: 90%;
    }
    .app-shell {
      min-height: 430px;
      grid-template-columns: 72px 1fr;
    }
    .app-shell aside {
      padding-inline: 12px;
    }
    .app-content {
      padding: 35px 20px;
    }
    .app-content h4 {
      font-size: 24px;
    }
    .shipping-sheet {
      top: 24%;
      left: 13%;
      width: 62%;
      min-height: 330px;
      padding: 25px;
    }
    .shipping-sheet .barcode {
      height: 70px;
      margin-top: 35px;
    }
    .shipping-sheet strong {
      font-size: 44px;
    }
    .route-chip,
    .driver-chip {
      right: 5%;
      bottom: 8%;
    }
    .product-label {
      top: auto;
      right: 5%;
      bottom: 8%;
      left: auto;
      width: 72%;
      min-height: 230px;
      padding: 25px 18px;
    }
    .product-label strong {
      font-size: 31px;
    }
    .service-board {
      top: 20%;
      left: 5%;
      width: 69%;
      padding: 20px;
    }
    .service-ticket {
      right: 4%;
      bottom: 9%;
      width: 43%;
      min-height: 300px;
      padding: 25px 16px;
    }
    .service-ticket strong {
      margin-top: 35px;
      font-size: 50px;
    }
    .locations-panel {
      top: 22%;
      left: 7%;
      width: 86%;
      padding: 23px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .workflow-stage,
    h3 button,
    h3 button > span {
      transition: none;
    }
  }
</style>

<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import type { ActionData, PageData } from './$types';

  let { data, form }: { data: PageData; form: ActionData } = $props();
  let code = $state('');
  const completed = $derived(form?.state === 'approved' || form?.state === 'denied');

  function formatCode(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const raw = input.value.toUpperCase().replace(/[^2-9A-HJ-NP-Z]/g, '').slice(0, 8);
    code = raw.length > 4 ? `${raw.slice(0, 4)}-${raw.slice(4)}` : raw;
  }
</script>

<svelte:head>
  <title>Pair a node · Spool</title>
  <meta name="robots" content="noindex,nofollow" />
</svelte:head>

<main>
  <a class="brand" href="/dashboard">
    <span><Icon name="printers" size={15} /></span>
    Spool
  </a>

  <section class="pair-card">
    {#if completed}
      <div class:denied={form?.state === 'denied'} class="result-icon">
        <Icon name={form?.state === 'approved' ? 'check' : 'x'} size={20} strokeWidth={2} />
      </div>
      <h1>{form?.state === 'approved' ? 'Node connected' : 'Pairing denied'}</h1>
      <p>
        {form?.state === 'approved'
          ? 'The device can now discover printers and sync its local queue. You can close this window.'
          : 'No node identity was issued. You can close this window.'}
      </p>
      {#if form?.state === 'approved'}
        <a class="button primary" href="/dashboard/nodes">View nodes</a>
      {/if}
    {:else if data.authorization}
      <div class="eyebrow">Browser pairing</div>
      <h1>Connect this node?</h1>
      <p>Only approve if the details match the computer where you started Spool.</p>

      <dl>
        <div><dt>Name</dt><dd>{data.authorization.proposed_name}</dd></div>
        <div><dt>Computer</dt><dd>{data.authorization.hostname}</dd></div>
        <div><dt>Platform</dt><dd>{data.authorization.platform} · {data.authorization.architecture}</dd></div>
      </dl>

      <form method="POST">
        <input type="hidden" name="authorization_id" value={data.authorization.id} />
        <label for="pairing-code">Code shown in the node</label>
        <input
          id="pairing-code"
          name="user_code"
          value={code}
          oninput={formatCode}
          placeholder="ABCD-2345"
          inputmode="text"
          autocomplete="one-time-code"
          autocapitalize="characters"
          spellcheck="false"
          maxlength="9"
          required
        />
        {#if form?.error}<p class="error" role="alert">{form.error}</p>{/if}
        <div class="actions">
          <button class="button" type="submit" formaction="?/deny">Deny</button>
          <button class="button primary" type="submit" formaction="?/approve" disabled={code.length !== 9}>
            Approve node
          </button>
        </div>
      </form>
    {:else}
      <div class="eyebrow">Browser pairing</div>
      <h1>Start from the Spool app</h1>
      <p>
        Open Spool on the printer computer and choose <strong>Connect node</strong>. The app will
        return here with a short-lived request.
      </p>
      {#if data.loadError}<p class="error" role="alert">{data.loadError}</p>{/if}
      <a class="button" href="/downloads">Download Spool</a>
    {/if}
  </section>

  <footer>Device codes expire after ten minutes and can be used only once.</footer>
</main>

<style>
  main {
    min-height: 100vh;
    display: grid;
    grid-template-rows: auto 1fr auto;
    justify-items: center;
    padding: 26px 18px 20px;
    background:
      radial-gradient(circle at 50% 28%, var(--accent-soft), transparent 32%),
      var(--canvas);
  }
  .brand { display: flex; align-items: center; gap: 8px; font-size: 13px; font-weight: 580; }
  .brand span { width: 26px; height: 26px; display: grid; place-items: center; color: white; background: var(--accent); border-radius: 7px; }
  .pair-card {
    align-self: center;
    width: min(100%, 410px);
    padding: 27px;
    background: color-mix(in oklch, var(--surface), transparent 5%);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-overlay);
    box-shadow: var(--shadow-overlay);
  }
  .eyebrow { margin-bottom: 9px; color: var(--accent); font-size: 9px; font-weight: 650; letter-spacing: .08em; text-transform: uppercase; }
  h1 { margin: 0; font-size: 21px; font-weight: 600; letter-spacing: -.035em; }
  p { margin: 7px 0 0; color: var(--text-secondary); font-size: 10px; line-height: 16px; }
  dl { margin: 22px 0; overflow: hidden; border: 1px solid var(--border-subtle); border-radius: 9px; }
  dl div { display: grid; grid-template-columns: 82px 1fr; gap: 12px; min-height: 38px; align-items: center; padding: 0 12px; border-bottom: 1px solid var(--border-subtle); }
  dl div:last-child { border-bottom: 0; }
  dt { color: var(--text-tertiary); font-size: 9px; }
  dd { margin: 0; overflow: hidden; color: var(--text-primary); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
  label { display: block; margin-bottom: 7px; color: var(--text-secondary); font-size: 9px; font-weight: 550; }
  input:not([type='hidden']) {
    width: 100%;
    height: 42px;
    padding: 0 12px;
    color: var(--text-primary);
    background: var(--surface-raised);
    border: 1px solid var(--border-default);
    border-radius: 8px;
    font: 600 17px/1 var(--font-mono);
    letter-spacing: .14em;
    text-align: center;
    outline: none;
  }
  input:focus { border-color: color-mix(in oklch, var(--accent), white 12%); box-shadow: 0 0 0 3px var(--accent-soft); }
  .actions { display: grid; grid-template-columns: 1fr 1.7fr; gap: 8px; margin-top: 16px; }
  .button { min-height: 35px; }
  .error { color: var(--danger); }
  .result-icon { width: 40px; height: 40px; display: grid; place-items: center; margin-bottom: 18px; color: var(--success); background: color-mix(in oklch, var(--success), transparent 88%); border-radius: 50%; }
  .result-icon.denied { color: var(--danger); background: color-mix(in oklch, var(--danger), transparent 88%); }
  .result-icon + h1 + p { margin-bottom: 20px; }
  footer { align-self: end; color: var(--text-tertiary); font-size: 9px; text-align: center; }
</style>

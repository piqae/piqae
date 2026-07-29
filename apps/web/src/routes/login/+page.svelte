<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import { createAuthBoundary } from '$lib/auth';
  import type { ActionData, PageData } from './$types';
  let { data, form }: { data: PageData; form: ActionData } = $props();
  const auth = createAuthBoundary('hosted');
</script>

<svelte:head><title>Sign in · Piqae</title></svelte:head>

<main>
  <section class="login-card">
    <a class="brand" href="/">
      <span><Icon name="printers" size={16} strokeWidth={2} /></span>
      Piqae
    </a>
    <div class="copy">
      <h1>Sign in to Piqae</h1>
      <p>Manage your print fleet, queues, API keys, and live delivery state.</p>
    </div>
    {#if data.authMode === 'local'}
      <form method="POST">
        <input type="hidden" name="return_to" value={data.returnTo} />
        <label for="credential">Owner credential</label>
        <input
          id="credential"
          name="credential"
          type="password"
          autocomplete="current-password"
          spellcheck="false"
          required
          placeholder="spl_owner_…"
        />
        {#if form?.invalid}
          <p class="form-error" role="alert">That owner credential was not accepted.</p>
        {/if}
        <button class="button primary sign-in" type="submit">
          Sign in <Icon name="arrow-right" size={13} />
        </button>
      </form>
      <p class="local-note">Your credential is exchanged server-side and stored only in an HttpOnly session cookie.</p>
    {:else}
      <a class="button primary sign-in" href={auth.signInUrl(data.returnTo)}>
        Continue with WorkOS <Icon name="arrow-right" size={13} />
      </a>
      <div class="divider"><span>or</span></div>
      <a class="button self-host" href="/docs/self-host">
        Configure self-hosted identity <Icon name="external" size={12} />
      </a>
    {/if}
    <p class="terms">
      By continuing, you agree to the service terms and acknowledge the privacy policy.
    </p>
  </section>
  <footer>
    <a href="/docs">Documentation</a>
    <a href="https://github.com/C4CoffeeCo/piqae">Open source</a>
    <a href="/docs/self-host">Self-host</a>
  </footer>
</main>

<style>
  main {
    min-height: 100vh;
    display: grid;
    grid-template-rows: 1fr auto;
    place-items: center;
    padding: 30px 18px 18px;
    background:
      radial-gradient(circle at 50% 25%, var(--accent-soft), transparent 34%),
      var(--canvas);
  }

  .login-card {
    width: min(100%, 370px);
    padding: 25px;
    background: color-mix(in oklch, var(--surface), transparent 8%);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-overlay);
    box-shadow: var(--shadow-overlay);
  }

  .brand {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    font-size: 14px;
    font-weight: 580;
  }

  .brand span {
    width: 27px;
    height: 27px;
    display: grid;
    place-items: center;
    color: white;
    background: var(--accent);
    border-radius: 7px;
  }

  .copy {
    padding: 27px 0 20px;
    text-align: center;
  }

  h1 {
    margin: 0;
    font-size: 19px;
    font-weight: 580;
    letter-spacing: -0.03em;
  }

  .copy p {
    margin: 6px auto 0;
    color: var(--text-secondary);
    font-size: 10px;
    line-height: 16px;
  }

  .sign-in,
  .self-host {
    width: 100%;
    min-height: 34px;
  }

  form {
    display: grid;
    gap: 8px;
  }

  label {
    color: var(--text-secondary);
    font-size: 10px;
    font-weight: 560;
  }

  input {
    width: 100%;
    min-height: 36px;
    padding: 0 11px;
    color: var(--text-primary);
    background: var(--surface);
    border: 1px solid var(--border-default);
    border-radius: 7px;
    font: inherit;
  }

  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .form-error {
    margin: 0;
    color: var(--danger, #d14b4b);
    font-size: 9px;
  }

  .local-note {
    margin: 11px 4px 0;
    color: var(--text-tertiary);
    font-size: 9px;
    line-height: 14px;
    text-align: center;
  }

  .divider {
    height: 35px;
    display: flex;
    align-items: center;
    color: var(--text-tertiary);
    font-size: 9px;
  }

  .divider::before,
  .divider::after {
    height: 1px;
    flex: 1;
    content: '';
    background: var(--border-subtle);
  }

  .divider span {
    padding: 0 9px;
  }

  .terms {
    margin: 18px 10px 0;
    color: var(--text-tertiary);
    font-size: 8px;
    line-height: 13px;
    text-align: center;
  }

  main > footer {
    display: flex;
    gap: 16px;
    padding-top: 30px;
  }

  footer a {
    color: var(--text-tertiary);
    font-size: 9px;
  }

  footer a:hover {
    color: var(--text-primary);
  }
</style>

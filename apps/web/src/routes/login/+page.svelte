<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import type { ActionData, PageData } from './$types';
  let { data, form }: { data: PageData; form: ActionData } = $props();
  let chosenStep = $state<string | null>(null);
  let step = $derived(chosenStep ?? form?.step ?? data.initialStep);
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
      <form method="POST" action="?/local">
        <input type="hidden" name="return_to" value={data.returnTo} />
        <label for="credential">Owner credential</label>
        <input
          id="credential"
          name="credential"
          type="password"
          autocomplete="current-password"
          spellcheck="false"
          required
          placeholder="piq_owner_…"
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
      {#if form?.notice}<p class="notice" role="status">{form.notice}</p>{/if}
      {#if step === 'password' || step === 'signup'}
        <form method="POST" action={step === 'signup' ? '?/signup' : '?/password'}>
          <input type="hidden" name="return_to" value={data.returnTo} />
          <label for="email">Email</label>
          <input id="email" name="email" type="email" autocomplete="email" required maxlength="320" />
          <label for="password">Password</label>
          <input
            id="password"
            name="password"
            type="password"
            autocomplete={step === 'signup' ? 'new-password' : 'current-password'}
            minlength={step === 'signup' ? 12 : undefined}
            required
          />
          {#if form?.invalid}
            <p class="form-error" role="alert">We couldn’t complete that request. Check the details and try again.</p>
          {/if}
          <button class="button primary sign-in" type="submit">
            {step === 'signup' ? 'Create account' : 'Sign in'} <Icon name="arrow-right" size={13} />
          </button>
        </form>
        <div class="inline-actions">
          <button type="button" onclick={() => (chosenStep = step === 'signup' ? 'password' : 'signup')}>
            {step === 'signup' ? 'Already have an account?' : 'Create account'}
          </button>
          {#if step === 'password'}
            <button type="button" onclick={() => (chosenStep = 'forgot')}>Forgot password?</button>
          {/if}
        </div>
        <div class="divider"><span>or</span></div>
        <button class="button self-host" type="button" onclick={() => (chosenStep = 'magic')}>
          Email me a sign-in code <Icon name="arrow-right" size={12} />
        </button>
      {:else if step === 'magic'}
        <form method="POST" action="?/magicStart">
          <input type="hidden" name="return_to" value={data.returnTo} />
          <label for="magic-email">Email</label>
          <input id="magic-email" name="email" type="email" autocomplete="email" required maxlength="320" />
          {#if form?.invalid}<p class="form-error" role="alert">We couldn’t send a code. Try again.</p>{/if}
          <button class="button primary sign-in" type="submit">Send code</button>
        </form>
        <button class="text-action" type="button" onclick={() => (chosenStep = 'password')}>Back to password</button>
      {:else if step === 'magic-code' || step === 'verify'}
        <form method="POST" action={step === 'verify' ? '?/verify' : '?/magicComplete'}>
          <input type="hidden" name="return_to" value={data.returnTo} />
          <label for="code">Six-digit code</label>
          <input id="code" name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" required />
          {#if form?.invalid}<p class="form-error" role="alert">That code was not accepted. Request a new code and try again.</p>{/if}
          <button class="button primary sign-in" type="submit">Verify and continue</button>
        </form>
        <button class="text-action" type="button" onclick={() => (chosenStep = 'password')}>Start again</button>
      {:else if step === 'mfa' || step === 'mfa-enroll'}
        {#if step === 'mfa-enroll' && form?.enrollment}
          <div class="mfa-enrollment">
            <img src={form.enrollment.qrCode} alt="QR code for adding Piqae to an authenticator app" width="160" height="160" />
            <p>Can’t scan it? Enter this setup key:</p>
            <code>{form.enrollment.secret}</code>
          </div>
        {/if}
        <form method="POST" action="?/mfa">
          <input type="hidden" name="return_to" value={data.returnTo} />
          <label for="mfa-code">Authenticator code</label>
          <input id="mfa-code" name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" required />
          {#if form?.invalid}<p class="form-error" role="alert">That code was not accepted. Check your authenticator and try again.</p>{/if}
          <button class="button primary sign-in" type="submit">Verify and continue</button>
        </form>
        <button class="text-action" type="button" onclick={() => (chosenStep = 'password')}>Start again</button>
      {:else if step === 'forgot'}
        <form method="POST" action="?/resetRequest">
          <input type="hidden" name="return_to" value={data.returnTo} />
          <label for="reset-email">Email</label>
          <input id="reset-email" name="email" type="email" autocomplete="email" required maxlength="320" />
          <button class="button primary sign-in" type="submit">Send reset instructions</button>
        </form>
        <button class="text-action" type="button" onclick={() => (chosenStep = 'password')}>Back to sign in</button>
      {:else if step === 'reset'}
        <form method="POST" action="?/reset">
          <input type="hidden" name="return_to" value={data.returnTo} />
          <input type="hidden" name="token" value={data.resetToken} />
          <label for="new-password">New password</label>
          <input id="new-password" name="password" type="password" autocomplete="new-password" minlength="12" required />
          {#if form?.invalid}<p class="form-error" role="alert">This reset link may have expired. Request another and try again.</p>{/if}
          <button class="button primary sign-in" type="submit">Update password</button>
        </form>
      {/if}
      <p class="hosted-fallback">
        Need SSO or additional verification?
        <a href={`/auth/login?hosted=1&return_to=${encodeURIComponent(data.returnTo)}`}>Use secure sign-in</a>
      </p>
    {/if}
    <p class="terms">
      By continuing, you agree to the service terms and acknowledge the privacy policy.
    </p>
  </section>
  <footer>
    <a href="/docs">Documentation</a>
    <a href="https://github.com/piqae/piqae">Open source</a>
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

  .notice {
    margin: 0 0 12px;
    padding: 9px 10px;
    color: var(--text-secondary);
    background: var(--surface-subtle);
    border-radius: 7px;
    font-size: 9px;
    line-height: 14px;
  }

  .inline-actions {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    margin-top: 10px;
  }

  .inline-actions button,
  .text-action {
    padding: 0;
    color: var(--accent);
    background: none;
    border: 0;
    font: inherit;
    font-size: 9px;
    cursor: pointer;
  }

  .text-action {
    display: block;
    margin: 13px auto 0;
  }

  .hosted-fallback {
    margin: 14px 0 0;
    color: var(--text-tertiary);
    font-size: 8px;
    line-height: 13px;
    text-align: center;
  }

  .hosted-fallback a { color: var(--accent); }

  .mfa-enrollment {
    display: grid;
    justify-items: center;
    gap: 8px;
    margin-bottom: 14px;
    text-align: center;
  }

  .mfa-enrollment img {
    width: 160px;
    height: 160px;
    background: white;
    border-radius: 8px;
  }

  .mfa-enrollment p { margin: 0; color: var(--text-secondary); font-size: 9px; }
  .mfa-enrollment code { overflow-wrap: anywhere; font-size: 10px; }

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

<script lang="ts">
  let { data } = $props();
</script>

<svelte:head>
  <title>Choose a workspace · Piqae</title>
  <meta
    name="description"
    content="Choose an existing Piqae workspace or create a new isolated workspace."
  />
</svelte:head>

<main>
  <div class="brand"><span aria-hidden="true">P</span> Piqae</div>
  <section aria-labelledby="onboarding-title">
    <header>
      <p class="eyebrow">Signed in as {data.user.email}</p>
      <h1 id="onboarding-title">Choose your workspace</h1>
      <p>Each workspace has isolated printers, nodes, jobs, API keys, and usage.</p>
    </header>

    {#if data.memberships.length > 0}
      <div class="workspace-list" aria-label="Your workspaces">
        {#each data.memberships as membership}
          <form method="POST" action="/auth/switch">
            <input type="hidden" name="organization_id" value={membership.organizationId} />
            <input type="hidden" name="return_to" value="/dashboard" />
            <button type="submit">
              <span>
                <strong>{membership.organizationName}</strong>
                <small>{membership.role}</small>
              </span>
              <span aria-hidden="true">→</span>
            </button>
          </form>
        {/each}
      </div>
      <div class="divider"><span>or</span></div>
    {/if}

    <form class="create" method="POST" action="/onboarding/workspace">
      <input type="hidden" name="workspace_token" value={data.workspaceToken} />
      <label for="workspace-name">Create a new workspace</label>
      <div class="input-row">
        <input
          id="workspace-name"
          name="name"
          minlength="2"
          maxlength="100"
          autocomplete="organization"
          placeholder="Acme Operations"
          required
        />
        <button type="submit">Create workspace</button>
      </div>
    </form>
  </section>
  <a class="sign-out" href="/auth/logout?return_to=/login">Sign out</a>
</main>

<style>
  :global(body) {
    margin: 0;
    background: var(--canvas, #0d0f12);
    color: var(--text-primary, #f5f6f7);
    font-family: var(--font-sans, system-ui, sans-serif);
  }

  main {
    width: min(540px, calc(100% - 32px));
    min-height: 100vh;
    display: grid;
    align-content: center;
    gap: 18px;
    margin: 0 auto;
    padding: 32px 0;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    font-weight: 650;
  }

  .brand span {
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    background: var(--accent, #5b7cfa);
    border-radius: 7px;
    color: white;
    font-size: 12px;
  }

  section {
    overflow: hidden;
    background: var(--surface-raised, #15181d);
    border: 1px solid var(--border-default, #2a2f37);
    border-radius: 14px;
    box-shadow: 0 18px 60px rgb(0 0 0 / 25%);
  }

  header,
  .create {
    padding: 24px;
  }

  .eyebrow {
    margin: 0 0 8px;
    color: var(--text-tertiary, #9299a5);
    font-size: 11px;
  }

  h1 {
    margin: 0;
    font-size: 24px;
    letter-spacing: -0.03em;
  }

  header > p:last-child {
    margin: 8px 0 0;
    color: var(--text-secondary, #b5bac4);
    font-size: 13px;
    line-height: 1.5;
  }

  .workspace-list {
    display: grid;
    border-top: 1px solid var(--border-subtle, #22262d);
  }

  .workspace-list form + form {
    border-top: 1px solid var(--border-subtle, #22262d);
  }

  .workspace-list button {
    width: 100%;
    min-height: 58px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 24px;
    background: transparent;
    border: 0;
    color: inherit;
    cursor: pointer;
    text-align: left;
  }

  .workspace-list button:hover,
  .workspace-list button:focus-visible {
    background: var(--surface-hover, #1c2026);
    outline: none;
  }

  .workspace-list button span:first-child {
    display: grid;
    gap: 3px;
  }

  strong,
  label {
    font-size: 12px;
    font-weight: 600;
  }

  small {
    color: var(--text-tertiary, #9299a5);
    font-size: 10px;
    text-transform: capitalize;
  }

  .divider {
    position: relative;
    height: 1px;
    background: var(--border-subtle, #22262d);
    text-align: center;
  }

  .divider span {
    position: relative;
    top: -8px;
    padding: 0 9px;
    background: var(--surface-raised, #15181d);
    color: var(--text-tertiary, #9299a5);
    font-size: 10px;
  }

  .create {
    display: grid;
    gap: 8px;
  }

  .input-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 8px;
  }

  input,
  .create button {
    min-height: 38px;
    box-sizing: border-box;
    border-radius: 8px;
    font: inherit;
  }

  input {
    min-width: 0;
    padding: 0 11px;
    background: var(--canvas, #0d0f12);
    border: 1px solid var(--border-default, #2a2f37);
    color: inherit;
    font-size: 12px;
  }

  input:focus {
    border-color: var(--accent, #5b7cfa);
    outline: 2px solid color-mix(in srgb, var(--accent, #5b7cfa) 30%, transparent);
  }

  .create button {
    padding: 0 14px;
    background: var(--accent, #5b7cfa);
    border: 1px solid var(--accent, #5b7cfa);
    color: white;
    cursor: pointer;
    font-size: 11px;
    font-weight: 600;
  }

  .sign-out {
    justify-self: center;
    color: var(--text-tertiary, #9299a5);
    font-size: 11px;
  }

  @media (max-width: 520px) {
    .input-row {
      grid-template-columns: 1fr;
    }
  }
</style>

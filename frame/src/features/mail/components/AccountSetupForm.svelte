<!--
  Adding a mail account: address and password (or a sign-in with Microsoft or
  Google in the system browser, with the client ID the person registered),
  servers found or typed by hand, a test of both servers, then the account.
  The password and a client secret stay in the setup object only until they
  are submitted and are never shown again.
-->
<script lang="ts">
  import { onDestroy } from "svelte";
  import * as m from "$shared/i18n/messages";
  import type { MailAccountV1, MailSecurityV1 } from "$shared/ipc/bindings";
  import { AccountSetup } from "$domain/mail";
  import Button from "$shared/ui/Button";
  import Field from "$shared/ui/Field";
  import Select from "$shared/ui/Select";
  import { oauthReasonText, reasonText } from "../lib/text";

  let {
    colourSeed = 0,
    onadded,
    oncancel,
  }: {
    /** Picks the account's starting colour, so each new account differs. */
    colourSeed?: number;
    onadded: (account: MailAccountV1) => void;
    oncancel?: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  const setup = new AccountSetup(colourSeed);
  onDestroy(() => setup.dispose());

  const security = [
    { value: "tls", label: m.mail_setup_tls() },
    { value: "start_tls", label: m.mail_setup_starttls() },
  ];

  const methods = $derived([
    { value: null, label: m.mail_setup_method_password() },
    { value: "microsoft" as const, label: m.mail_oauth_microsoft() },
    { value: "google" as const, label: m.mail_oauth_google() },
  ]);
  const oauth = $derived(setup.oauthProvider !== null);

  async function add() {
    const account = await setup.add();
    if (account) onadded(account);
  }
</script>

<form
  class="setup"
  aria-labelledby="mail-setup-title"
  onsubmit={(event) => {
    event.preventDefault();
    if (setup.canAdd) void add();
    else if (setup.canTest && !setup.tested) void setup.runTest();
    else if (oauth && setup.canSignIn && !setup.grant) void setup.signIn();
    else if (!oauth && setup.canDetect && !setup.serversReady) void setup.detect();
  }}
>
  <h2 id="mail-setup-title">{m.mail_setup_title()}</h2>
  <p class="intro">{m.mail_setup_intro()}</p>

  <div class="methods" role="group" aria-label={m.mail_setup_method()}>
    {#each methods as method (method.value)}
      <Button
        type="button"
        size="compact"
        variant={setup.oauthProvider === method.value ? "primary" : "ghost"}
        aria-pressed={setup.oauthProvider === method.value}
        disabled={setup.busy}
        onclick={() => setup.chooseProvider(method.value)}>{method.label}</Button
      >
    {/each}
  </div>

  <div class="fields">
    <Field
      label={m.mail_setup_email()}
      type="email"
      autocomplete="email"
      spellcheck={false}
      bind:value={setup.email}
      oninput={() => setup.touched()}
    />
    {#if !oauth}
      <Field
        label={m.mail_setup_password()}
        type="password"
        autocomplete="off"
        hint={setup.passwordCleared
          ? m.mail_setup_password_cleared()
          : m.mail_setup_password_hint()}
        bind:value={setup.password}
        oninput={() => {
          setup.touched();
          setup.passwordCleared = false;
        }}
      />
    {/if}
    <Field label={m.mail_setup_name()} autocomplete="name" bind:value={setup.displayName} />
  </div>

  {#if setup.oauthProvider}
    <section class="oauth" aria-label={m.mail_oauth_section()}>
      <p class="note">{m.mail_oauth_help()}</p>
      <Field
        label={m.mail_oauth_client_id()}
        hint={m.mail_oauth_client_id_hint()}
        autocomplete="off"
        spellcheck={false}
        disabled={setup.busy}
        bind:value={setup.clientId}
        oninput={() => setup.touched()}
      />
      {#if setup.oauthProvider === "microsoft"}
        <Field
          label={m.mail_oauth_tenant()}
          hint={m.mail_oauth_tenant_hint()}
          autocomplete="off"
          spellcheck={false}
          disabled={setup.busy}
          bind:value={setup.tenant}
        />
      {:else}
        <Field
          label={m.mail_oauth_secret()}
          type="password"
          autocomplete="off"
          hint={setup.secretCleared ? m.mail_oauth_secret_cleared() : m.mail_oauth_secret_hint()}
          disabled={setup.busy}
          bind:value={setup.clientSecret}
          oninput={() => (setup.secretCleared = false)}
        />
      {/if}
      <div class="row">
        <Button
          variant="primary"
          type="button"
          disabled={!setup.canSignIn}
          pending={setup.phase === "signing_in"}
          onclick={() => void setup.signIn()}
        >
          {setup.phase === "signing_in"
            ? m.mail_oauth_signing_in()
            : setup.oauthProvider === "microsoft"
              ? m.mail_oauth_sign_in_microsoft()
              : m.mail_oauth_sign_in_google()}
        </Button>
        {#if setup.phase === "signing_in"}
          <Button variant="ghost" type="button" onclick={() => setup.cancelSignIn()}>
            {m.mail_oauth_cancel()}
          </Button>
        {/if}
      </div>
      {#if setup.oauthFailure}
        <p class="problem" role="alert">
          {m.mail_oauth_failed({ reason: oauthReasonText(setup.oauthFailure) })}
        </p>
      {:else if setup.grant}
        <p class="good" role="status">{m.mail_oauth_signed_in()}</p>
      {/if}
      <p class="note">{m.mail_oauth_servers()}</p>
    </section>
  {/if}

  {#if !oauth && !setup.serversReady && !setup.manual}
    <div class="row">
      <Button
        variant="primary"
        type="button"
        disabled={!setup.canDetect}
        pending={setup.phase === "detecting"}
        onclick={() => void setup.detect()}
      >
        {setup.phase === "detecting" ? m.mail_setup_finding() : m.mail_setup_find()}
      </Button>
      <Button
        variant="ghost"
        type="button"
        disabled={setup.busy}
        onclick={() => setup.enterManually()}
      >
        {m.mail_setup_manual()}
      </Button>
    </div>
  {/if}

  {#if oauth}
    <!-- The provider fixes the servers: nothing to find or type. -->
  {:else if setup.detected && !setup.detected.found}
    <p class="note" role="status">{m.mail_setup_not_found()}</p>
  {:else if setup.detected?.found && !setup.manual}
    <p class="note" role="status">
      {m.mail_setup_found()}{setup.detected.provider ? `: ${setup.detected.provider}` : ""}
    </p>
  {/if}

  {#if !oauth && (setup.serversReady || setup.manual)}
    <fieldset class="servers" disabled={setup.busy}>
      <legend>{m.mail_setup_incoming()}</legend>
      <Field
        label={m.mail_setup_host()}
        spellcheck={false}
        bind:value={setup.imap.host}
        oninput={() => setup.touched()}
      />
      <Field
        label={m.mail_setup_port()}
        inputmode="numeric"
        bind:value={setup.imap.port}
        oninput={() => setup.touched()}
      />
      <Select
        label={m.mail_setup_security()}
        options={security}
        value={setup.imap.security}
        onchange={(value) => {
          setup.imap.security = value as MailSecurityV1;
          setup.touched();
        }}
      />
    </fieldset>
    <fieldset class="servers" disabled={setup.busy}>
      <legend>{m.mail_setup_outgoing()}</legend>
      <Field
        label={m.mail_setup_host()}
        spellcheck={false}
        bind:value={setup.smtp.host}
        oninput={() => setup.touched()}
      />
      <Field
        label={m.mail_setup_port()}
        inputmode="numeric"
        bind:value={setup.smtp.port}
        oninput={() => setup.touched()}
      />
      <Select
        label={m.mail_setup_security()}
        options={security}
        value={setup.smtp.security}
        onchange={(value) => {
          setup.smtp.security = value as MailSecurityV1;
          setup.touched();
        }}
      />
    </fieldset>
    <Field
      label={m.mail_setup_username()}
      autocomplete="username"
      spellcheck={false}
      bind:value={setup.username}
      oninput={() => setup.touched()}
    />
  {/if}

  <div class="status" aria-live="polite">
    {#if setup.failure}
      <p class="problem" role="alert">{reasonText(setup.failure)}</p>
    {/if}
    {#if setup.test}
      {#if setup.tested}
        <p class="good">{m.mail_setup_test_ok()}</p>
      {:else}
        {#if setup.test.imap}
          <p class="problem" role="alert">
            {m.mail_setup_test_imap_failed({ reason: reasonText(setup.test.imap) })}
          </p>
        {/if}
        {#if setup.test.smtp}
          <p class="problem" role="alert">
            {m.mail_setup_test_smtp_failed({ reason: reasonText(setup.test.smtp) })}
          </p>
        {/if}
      {/if}
    {/if}
  </div>

  {#if oauth || setup.serversReady || setup.manual}
    <div class="row">
      <Button
        type="button"
        disabled={!setup.canTest}
        pending={setup.phase === "testing"}
        onclick={() => void setup.runTest()}
      >
        {setup.phase === "testing" ? m.mail_setup_testing() : m.mail_setup_test()}
      </Button>
      <Button
        variant="primary"
        type="submit"
        disabled={!setup.canAdd}
        pending={setup.phase === "adding"}
      >
        {setup.phase === "adding" ? m.mail_setup_adding() : m.mail_setup_add()}
      </Button>
      {#if oncancel}
        <Button variant="ghost" type="button" disabled={setup.busy} onclick={oncancel}>
          {m.mail_settings_cancel()}
        </Button>
      {/if}
    </div>
  {:else if oncancel}
    <div class="row">
      <Button variant="ghost" type="button" onclick={oncancel}>{m.mail_settings_cancel()}</Button>
    </div>
  {/if}
</form>

<style>
  .setup {
    display: grid;
    gap: 16px;
    inline-size: min(100%, 460px);
    margin-inline: auto;
    padding: 32px 16px;
    color: var(--color-text);
  }

  h2 {
    margin: 0;
    font-size: var(--text-title);
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .intro,
  .note {
    margin: 0;
    color: var(--color-muted);
    font-size: var(--text-body);
    line-height: 1.5;
  }

  .fields {
    display: grid;
    gap: 12px;
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .methods {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .oauth {
    display: grid;
    gap: 12px;
    padding: 12px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
  }

  .servers {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 88px;
    gap: 8px 12px;
    margin: 0;
    padding: 12px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
    min-inline-size: 0;
  }

  .servers legend {
    padding-inline: 4px;
    color: var(--color-muted);
    font-size: var(--text-label);
    font-weight: 500;
  }

  .servers > :global(:last-child) {
    grid-column: 1 / -1;
  }

  .status {
    display: grid;
    gap: 4px;
    min-block-size: 0;
  }

  .status p {
    margin: 0;
    font-size: var(--text-body);
  }

  .problem {
    color: var(--color-danger);
  }

  .good {
    color: var(--color-text);
  }

  .oauth .note,
  .oauth .problem,
  .oauth .good {
    margin: 0;
    font-size: var(--text-body);
    line-height: 1.5;
  }
</style>

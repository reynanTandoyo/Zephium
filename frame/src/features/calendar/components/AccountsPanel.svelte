<!--
  Accounts: add a CalDAV account (address, then sign-in), subscribe to an ICS
  address, remove an account. The password lives in this form only while it is
  typed: it is cleared the moment it is submitted, and never shown or kept.
-->
<script lang="ts">
  import { Cancel01Icon } from "@hugeicons/core-free-icons";
  import * as m from "$shared/i18n/messages";
  import Button from "$shared/ui/Button";
  import Field from "$shared/ui/Field";
  import IconButton from "$shared/ui/IconButton";
  import { calendar } from "$domain/calendar";
  import type { CalendarFault } from "$domain/calendar";
  import { faultText } from "../lib/format";

  let { onclose }: { onclose: () => void } = $props();

  type Step = "list" | "address" | "sign-in" | "subscribe";
  let step = $state<Step>("list");
  let address = $state("");
  let serverUrl = $state("");
  let username = $state("");
  let password = $state("");
  let name = $state("");
  let feedUrl = $state("");
  let fault = $state<CalendarFault | null>(null);
  let finding = $state(false);
  let removing = $state<string | null>(null);

  const adding = $derived(calendar.busy() === "account");

  function go(next: Step) {
    step = next;
    fault = null;
    password = "";
  }

  async function find(event: SubmitEvent) {
    event.preventDefault();
    if (finding || address.trim() === "") return;
    finding = true;
    fault = null;
    const result = await calendar.discover(address);
    finding = false;
    if (!result.ok) {
      fault = result.error;
      return;
    }
    serverUrl = result.value.server_url;
    username = result.value.username;
    step = "sign-in";
  }

  async function signIn(event: SubmitEvent) {
    event.preventDefault();
    if (adding) return;
    // The secret leaves the form before the call, win or lose.
    const secret = password;
    password = "";
    fault = null;
    const result = await calendar.addAccount(serverUrl, username, secret, name.trim() || null);
    if (!result.ok) {
      fault = result.error;
      return;
    }
    address = serverUrl = username = name = "";
    go("list");
  }

  async function subscribe(event: SubmitEvent) {
    event.preventDefault();
    if (adding) return;
    fault = null;
    const result = await calendar.addSubscription(feedUrl, name);
    if (!result.ok) {
      fault = result.error;
      return;
    }
    feedUrl = name = "";
    go("list");
  }

  async function remove(id: string) {
    fault = null;
    const result = await calendar.removeAccount(id);
    removing = null;
    if (!result.ok) fault = result.error;
  }
</script>

<section class="accounts" aria-label={m.calendar_accounts_title()}>
  <header>
    <h2>{m.calendar_accounts_title()}</h2>
    <IconButton
      icon={Cancel01Icon}
      label={m.calendar_close()}
      size={14}
      buttonSize={28}
      onclick={onclose}
    />
  </header>

  {#if step === "list"}
    {#if calendar.accounts().length === 0}
      <p class="note">{m.calendar_no_accounts()}</p>
    {:else}
      <ul class="list">
        {#each calendar.accounts() as account (account.id)}
          <li>
            <div class="who">
              <strong>{account.name}</strong>
              <span class="kind">
                {account.kind === "caldav"
                  ? m.calendar_account_caldav()
                  : m.calendar_account_subscription()}
                {#if account.username}· {account.username}{/if}
              </span>
            </div>
            {#if removing === account.id}
              <p class="note" role="alert">{m.calendar_remove_confirm({ name: account.name })}</p>
              <div class="actions">
                <Button variant="danger" pending={adding} onclick={() => void remove(account.id)}>
                  {m.calendar_remove()}
                </Button>
                <Button disabled={adding} onclick={() => (removing = null)}
                  >{m.calendar_cancel()}</Button
                >
              </div>
            {:else}
              <Button variant="ghost" size="compact" onclick={() => (removing = account.id)}>
                {m.calendar_remove()}
              </Button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if fault}<p class="error" role="alert">{faultText(fault)}</p>{/if}
    <div class="actions">
      <Button variant="primary" onclick={() => go("address")}>{m.calendar_add_caldav()}</Button>
      <Button onclick={() => go("subscribe")}>{m.calendar_add_subscription()}</Button>
    </div>
  {:else if step === "address"}
    <form onsubmit={find} aria-label={m.calendar_add_caldav()}>
      <Field
        label={m.calendar_field_address()}
        hint={m.calendar_field_address_hint()}
        bind:value={address}
        autocomplete="off"
        autocapitalize="off"
        spellcheck={false}
        required
      />
      {#if fault}<p class="error" role="alert">{faultText(fault)}</p>{/if}
      <div class="actions">
        <Button type="submit" variant="primary" pending={finding}>{m.calendar_continue()}</Button>
        <Button disabled={finding} onclick={() => go("list")}>{m.calendar_cancel()}</Button>
      </div>
    </form>
  {:else if step === "sign-in"}
    <form onsubmit={signIn} aria-label={m.calendar_add_caldav()}>
      <Field label={m.calendar_field_server()} bind:value={serverUrl} autocomplete="off" required />
      <Field
        label={m.calendar_field_username()}
        bind:value={username}
        autocomplete="username"
        required
      />
      <Field
        label={m.calendar_field_password()}
        type="password"
        bind:value={password}
        autocomplete="current-password"
        required
      />
      <Field label={m.calendar_field_name()} bind:value={name} autocomplete="off" />
      {#if fault}<p class="error" role="alert">{faultText(fault)}</p>{/if}
      <div class="actions">
        <Button type="submit" variant="primary" pending={adding}>{m.calendar_sign_in()}</Button>
        <Button disabled={adding} onclick={() => go("address")}>{m.calendar_back()}</Button>
      </div>
    </form>
  {:else}
    <form onsubmit={subscribe} aria-label={m.calendar_add_subscription()}>
      <Field
        label={m.calendar_field_feed()}
        hint={m.calendar_field_feed_hint()}
        bind:value={feedUrl}
        autocomplete="off"
        autocapitalize="off"
        spellcheck={false}
        required
      />
      <Field label={m.calendar_field_name()} bind:value={name} autocomplete="off" required />
      {#if fault}<p class="error" role="alert">{faultText(fault)}</p>{/if}
      <div class="actions">
        <Button type="submit" variant="primary" pending={adding}>{m.calendar_subscribe()}</Button>
        <Button disabled={adding} onclick={() => go("list")}>{m.calendar_cancel()}</Button>
      </div>
    </form>
  {/if}
</section>

<style>
  .accounts {
    display: grid;
    align-content: start;
    gap: 12px;
    padding: 16px;
    color: var(--color-text);
  }

  header {
    display: flex;
    align-items: center;
  }

  h2 {
    flex: 1;
    margin: 0;
    font-size: var(--text-title);
    font-weight: 600;
  }

  form {
    display: grid;
    gap: 12px;
  }

  .list {
    display: grid;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .list > li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: var(--radius-row);
    background: var(--color-fill);
  }

  .who {
    display: grid;
    flex: 1;
    min-inline-size: 0;
  }

  strong {
    overflow: hidden;
    font-size: var(--text-body);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kind,
  .note {
    margin: 0;
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  .error {
    margin: 0;
    color: var(--color-danger);
    font-size: var(--text-label);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
</style>

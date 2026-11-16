<!-- One account's name, colour and signature, its place in the list, and removing it after a confirmation. -->
<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import type { MailAccountV1 } from "$shared/ipc/bindings";
  import type { MailStore } from "$domain/mail";
  import Button from "$shared/ui/Button";
  import ColorWell from "$shared/ui/ColorWell";
  import Field from "$shared/ui/Field";
  import TextArea from "$shared/ui/TextArea";

  let {
    store,
    account,
    index,
    count,
  }: { store: MailStore; account: MailAccountV1; index: number; count: number } = $props();
  let moving = $state(false);

  async function move(step: -1 | 1) {
    moving = true;
    await store.moveAccount(account.id, step);
    moving = false;
  }

  // The form starts from the account and is the person's own until saved.
  // svelte-ignore state_referenced_locally
  let name = $state(account.display_name);
  // svelte-ignore state_referenced_locally
  let colour = $state(account.colour);
  // svelte-ignore state_referenced_locally
  let signature = $state(account.signature);
  let removing = $state(false);
  let saving = $state(false);
  let saved = $state(false);
  let removingNow = $state(false);

  const dirty = $derived(
    name !== account.display_name || colour !== account.colour || signature !== account.signature,
  );

  async function save() {
    saving = true;
    saved = false;
    const ok = await store.updateAccount({
      id: account.id,
      display_name: name.trim() || account.email,
      email: account.email,
      imap: account.imap,
      smtp: account.smtp,
      username: account.username,
      colour,
      signature,
      save_sent_copy: account.save_sent_copy,
      password: null,
    });
    saving = false;
    saved = ok;
  }

  async function remove() {
    removingNow = true;
    const ok = await store.removeAccount(account.id);
    removingNow = false;
    if (ok) removing = false;
  }
</script>

<li class="account">
  <div class="head">
    <h3>{account.email}</h3>
    {#if account.sign_in_failed}
      <p class="signin" role="status">{m.mail_sign_in_failed()}</p>
    {/if}
    <div class="bar" role="group" aria-label={m.mail_settings_order({ email: account.email })}>
      <Button
        size="compact"
        disabled={index === 0 || moving}
        aria-label={m.mail_settings_move_up_for({ email: account.email })}
        onclick={() => void move(-1)}>{m.mail_settings_move_up()}</Button
      >
      <Button
        size="compact"
        disabled={index === count - 1 || moving}
        aria-label={m.mail_settings_move_down_for({ email: account.email })}
        onclick={() => void move(1)}>{m.mail_settings_move_down()}</Button
      >
    </div>
  </div>
  <div class="grid">
    <Field label={m.mail_settings_name()} bind:value={name} />
    <ColorWell
      label={m.mail_settings_colour()}
      hexLabel={m.mail_settings_colour_hex()}
      invalidLabel={m.mail_settings_colour_invalid()}
      value={colour}
      onchange={(next) => (colour = next)}
    />
  </div>
  <TextArea label={m.mail_settings_signature()} rows={3} bind:value={signature} />
  <div class="bar">
    <Button variant="primary" disabled={!dirty} pending={saving} onclick={() => void save()}>
      {m.mail_settings_save()}
    </Button>
    <Button variant="danger" onclick={() => (removing = true)}>{m.mail_settings_remove()}</Button>
    {#if saved && !dirty}<span class="muted" role="status">{m.mail_settings_saved()}</span>{/if}
  </div>
  {#if removing}
    <div class="confirm" role="alertdialog" aria-label={m.mail_settings_remove()}>
      <p>{m.mail_settings_remove_confirm({ email: account.email })}</p>
      <div class="bar">
        <Button variant="danger" pending={removingNow} onclick={() => void remove()}>
          {m.mail_settings_remove_yes()}
        </Button>
        <Button disabled={removingNow} onclick={() => (removing = false)}
          >{m.mail_settings_cancel()}</Button
        >
      </div>
    </div>
  {/if}
</li>

<style>
  .account {
    display: grid;
    gap: 12px;
    padding: 16px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
    list-style: none;
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .signin {
    margin: 0;
    color: var(--color-danger);
    font-size: var(--text-label);
  }

  h3 {
    margin: 0;
    font-size: var(--text-page-title);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 12px;
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .confirm {
    display: grid;
    gap: 8px;
    padding: 12px;
    border-radius: var(--radius-control);
    background: var(--color-fill);
  }

  .confirm p {
    margin: 0;
    font-size: var(--text-body);
    line-height: 1.5;
  }

  .muted {
    color: var(--color-muted);
    font-size: var(--text-label);
  }
</style>

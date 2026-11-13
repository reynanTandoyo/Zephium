<!--
  Mail settings: the accounts with their display name, colour and signature,
  their order, and removing one after a confirmation. The server and password are not
  edited here; removing and adding the account again changes them.
-->
<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import type { MailStore } from "$domain/mail";
  import Button from "$shared/ui/Button";
  import { reasonText } from "../lib/text";
  import AccountEditor from "./AccountEditor.svelte";
  import RuleEditor from "./RuleEditor.svelte";

  let { store, onadd, onback }: { store: MailStore; onadd: () => void; onback: () => void } =
    $props();
</script>

<section class="settings" aria-labelledby="mail-settings-title">
  <header>
    <h2 id="mail-settings-title">{m.mail_settings_title()}</h2>
    <div class="bar">
      <Button variant="primary" onclick={onadd}>{m.mail_settings_add()}</Button>
      <Button variant="ghost" onclick={onback}>{m.mail_settings_back()}</Button>
    </div>
  </header>

  {#if store.notice?.action === "account" || store.notice?.action === "order"}
    <p class="bad" role="alert">
      {m.mail_notice_failed({ reason: reasonText(store.notice.error) })}
    </p>
  {/if}

  <ul>
    {#each store.accounts as account, index (account.id)}
      <AccountEditor {store} {account} {index} count={store.accounts.length} />
    {/each}
  </ul>

  {#if store.accounts.length}<RuleEditor {store} />{/if}
</section>

<style>
  .settings {
    display: grid;
    align-content: start;
    gap: 16px;
    inline-size: min(100%, 640px);
    margin-inline: auto;
    padding: 24px 16px;
    color: var(--color-text);
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  h2 {
    margin: 0;
    font-size: var(--text-title);
    font-weight: 600;
  }

  ul {
    display: grid;
    gap: 12px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .bad {
    margin: 0;
    color: var(--color-danger);
    font-size: var(--text-body);
  }
</style>

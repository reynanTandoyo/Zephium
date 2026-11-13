<!--
  Writing a message: new, reply, reply all or forward. Plain text, a From
  picker over every account, attachments from the file picker. Sent is shown
  only after the server accepted the message.
-->
<script lang="ts">
  import Attachment01Icon from "@hugeicons/core-free-icons/Attachment01Icon";
  import Cancel01Icon from "@hugeicons/core-free-icons/Cancel01Icon";
  import type { ComposeSession, MailStore } from "$domain/mail";
  import * as m from "$shared/i18n/messages";
  import Button from "$shared/ui/Button";
  import Field from "$shared/ui/Field";
  import Icon from "$shared/ui/Icon";
  import IconButton from "$shared/ui/IconButton";
  import Select from "$shared/ui/Select";
  import TextArea from "$shared/ui/TextArea";
  import { formatSize, reasonText } from "../lib/text";
  import AddressField from "./AddressField.svelte";

  let {
    store,
    compose,
    onclose,
  }: { store: MailStore; compose: ComposeSession; onclose: () => void } = $props();

  let showCopies = $state(false);
  let confirming = $state(false);
  let picker: HTMLInputElement | undefined = $state();

  const accounts = $derived(
    store.accounts.map((account) => ({
      value: account.id,
      label: `${account.display_name} <${account.email}>`,
    })),
  );
  const copiesShown = $derived(showCopies || compose.cc.length > 0 || compose.bcc.length > 0);

  function close() {
    if (compose.dirty && compose.state !== "sent") confirming = true;
    else onclose();
  }

  async function attach(files: FileList | null) {
    if (!files) return;
    for (const file of files) await compose.attach(file);
    if (picker) picker.value = "";
  }

  const problemText = $derived.by(() => {
    switch (compose.problem) {
      case "no_recipient":
        return m.mail_problem_no_recipient();
      case "invalid_address":
        return m.mail_problem_invalid_address({ list: compose.invalid.join(", ") });
      case "too_many":
        return m.mail_problem_too_many();
      case "too_large":
        return m.mail_problem_too_large();
      case "attachment_too_large":
        return m.mail_problem_attachment_too_large();
      default:
        return null;
    }
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section
  class="compose"
  aria-label={m.mail_compose_title()}
  onkeydown={(event) => {
    if (event.key === "Escape" && !confirming) {
      event.stopPropagation();
      close();
    } else if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      void compose.send();
    }
  }}
>
  <header>
    <h2>{m.mail_compose_title()}</h2>
    <IconButton icon={Cancel01Icon} label={m.mail_compose_close()} onclick={close} />
  </header>

  {#if compose.state === "sent"}
    <div class="done" role="status">
      <p>{m.mail_compose_sent()}</p>
      {#if !compose.savedCopy}<p class="muted">{m.mail_compose_no_sent_copy()}</p>{/if}
      <Button variant="primary" onclick={onclose}>{m.mail_compose_close()}</Button>
    </div>
  {:else}
    <form
      onsubmit={(event) => {
        event.preventDefault();
        void compose.send();
      }}
    >
      <div class="fields">
        {#if accounts.length > 1}
          <Select
            label={m.mail_compose_from()}
            options={accounts}
            value={compose.account}
            onchange={(id) => compose.setAccount(id)}
            disabled={compose.busy}
          />
        {/if}
        <AddressField
          label={m.mail_compose_to()}
          bind:value={compose.to}
          oninput={() => compose.edited()}
          disabled={compose.busy}
        />
        {#if copiesShown}
          <AddressField
            label={m.mail_compose_cc()}
            bind:value={compose.cc}
            oninput={() => compose.edited()}
            disabled={compose.busy}
          />
          <AddressField
            label={m.mail_compose_bcc()}
            bind:value={compose.bcc}
            oninput={() => compose.edited()}
            disabled={compose.busy}
          />
        {:else}
          <Button variant="ghost" size="compact" onclick={() => (showCopies = true)}
            >{m.mail_compose_show_cc()}</Button
          >
        {/if}
        <Field
          label={m.mail_compose_subject()}
          bind:value={compose.subject}
          oninput={() => compose.edited()}
          disabled={compose.busy}
        />
        <TextArea
          label={m.mail_compose_body()}
          rows={14}
          bind:value={compose.text}
          oninput={() => compose.edited()}
          disabled={compose.busy}
        />
      </div>

      {#if compose.attachments.length}
        <ul class="files" aria-label={m.mail_attachments()}>
          {#each compose.attachments as attachment, at (at)}
            <li>
              <Icon icon={Attachment01Icon} size={14} />
              <span class="name">{attachment.filename}</span>
              <span class="size">{formatSize(attachment.size)}</span>
              <IconButton
                icon={Cancel01Icon}
                label={m.mail_compose_remove_attachment({ name: attachment.filename })}
                onclick={() => compose.detach(at)}
              />
            </li>
          {/each}
        </ul>
      {/if}

      <div class="status" aria-live="polite">
        {#if problemText}<p class="bad" role="alert">{problemText}</p>{/if}
        {#if compose.state === "failed" && compose.failure}
          <p class="bad" role="alert">
            {compose.unknownOutcome
              ? m.mail_send_unknown()
              : m.mail_send_failed({ reason: reasonText(compose.failure) })}
          </p>
        {/if}
        {#if compose.saved && compose.state === "editing"}<p class="muted">
            {m.mail_compose_saved()}
          </p>{/if}
      </div>

      <div class="bar">
        <Button
          variant="primary"
          type="submit"
          pending={compose.state === "sending"}
          disabled={compose.busy}
        >
          {compose.state === "sending" ? m.mail_compose_sending() : m.mail_compose_send()}
        </Button>
        <Button
          pending={compose.state === "saving"}
          disabled={compose.busy}
          onclick={() => void compose.saveDraft()}
        >
          {compose.state === "saving" ? m.mail_compose_saving() : m.mail_compose_save()}
        </Button>
        <Button variant="ghost" disabled={compose.busy} onclick={() => picker?.click()}>
          {m.mail_compose_attach()}
        </Button>
        <input
          bind:this={picker}
          class="picker"
          type="file"
          multiple
          tabindex="-1"
          aria-hidden="true"
          onchange={(event) => void attach(event.currentTarget.files)}
        />
      </div>
    </form>
  {/if}

  {#if confirming}
    <div class="confirm" role="alertdialog" aria-labelledby="mail-discard-title">
      <p id="mail-discard-title">{m.mail_compose_discard_title()}</p>
      <div class="bar">
        <Button variant="danger" onclick={onclose}>{m.mail_compose_discard()}</Button>
        <Button onclick={() => (confirming = false)}>{m.mail_compose_keep()}</Button>
      </div>
    </div>
  {/if}
</section>

<style>
  .compose {
    position: relative;
    display: flex;
    flex-direction: column;
    min-block-size: 0;
    min-inline-size: 0;
    overflow-y: auto;
    color: var(--color-text);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 20px;
    border-block-end: 1px solid var(--color-border);
  }

  h2 {
    margin: 0;
    font-size: var(--text-page-title);
    font-weight: 600;
  }

  form {
    display: grid;
    gap: 12px;
    padding: 16px 20px;
  }

  .fields {
    display: grid;
    gap: 10px;
  }

  .fields :global(textarea) {
    font-family: var(--font-mono);
    min-block-size: 220px;
  }

  .files {
    display: grid;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-label);
  }

  .files li {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size,
  .muted {
    color: var(--color-faint);
    font-size: var(--text-label);
  }

  .status p {
    margin: 0;
  }

  .bad {
    color: var(--color-danger);
    font-size: var(--text-body);
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .picker {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    opacity: 0;
    pointer-events: none;
  }

  .done {
    display: grid;
    justify-items: start;
    gap: 12px;
    padding: 24px 20px;
  }

  .done p {
    margin: 0;
  }

  .confirm {
    position: absolute;
    inset-block-start: 56px;
    inset-inline: 20px;
    display: grid;
    gap: 12px;
    padding: 16px;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-card);
    background: var(--color-raised);
  }

  .confirm p {
    margin: 0;
    font-weight: 550;
  }
</style>

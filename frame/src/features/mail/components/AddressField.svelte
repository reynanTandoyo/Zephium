<!--
  A recipient field with suggestions. While an address is typed, the people
  you have written to or heard from (from mail already on this computer; no
  network) are listed below. Arrow keys move through them, Enter or Tab
  takes the highlighted one, Escape closes the list.
-->
<script lang="ts">
  import { onDestroy } from "svelte";
  import * as m from "$shared/i18n/messages";
  import type { MailContactV1 } from "$shared/ipc/bindings";
  import { completeEntry, ContactSuggester, displayName, parseAddresses } from "$domain/mail";
  import Field from "$shared/ui/Field";

  let {
    label,
    value = $bindable(""),
    disabled = false,
    oninput,
  }: { label: string; value?: string; disabled?: boolean; oninput?: () => void } = $props();

  const suggester = new ContactSuggester();
  const listId = $props.id();
  let active = $state(-1);
  let focused = $state(false);
  let closed = $state(false);

  /** Addresses already in the field are not offered again. */
  const entered = $derived(
    new Set(parseAddresses(value).addresses.map((address) => address.email.toLowerCase())),
  );
  const options = $derived(
    suggester.results.filter((contact) => !entered.has(contact.email.toLowerCase())),
  );
  const open = $derived(focused && !closed && options.length > 0);

  $effect(() => {
    void options;
    active = -1;
  });

  onDestroy(() => suggester.dispose());

  function typed(event: Event) {
    value = (event.currentTarget as HTMLInputElement).value;
    closed = false;
    suggester.request(value);
    oninput?.();
  }

  function pick(contact: MailContactV1) {
    value = completeEntry(value, contact);
    suggester.clear();
    oninput?.();
  }

  function keydown(event: KeyboardEvent) {
    if (!open) return;
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        active = (active + 1) % options.length;
        break;
      case "ArrowUp":
        event.preventDefault();
        active = active <= 0 ? options.length - 1 : active - 1;
        break;
      case "Enter":
      case "Tab": {
        const chosen = options[active];
        if (!chosen) return;
        event.preventDefault();
        pick(chosen);
        break;
      }
      case "Escape":
        // Closes the list only; the message stays open.
        event.stopPropagation();
        closed = true;
        break;
    }
  }
</script>

<div class="address">
  <Field
    {label}
    spellcheck={false}
    autocomplete="off"
    role="combobox"
    aria-autocomplete="list"
    aria-expanded={open}
    aria-controls={listId}
    aria-activedescendant={open && active >= 0 ? `${listId}-${active}` : undefined}
    bind:value
    oninput={typed}
    onkeydown={keydown}
    onfocus={() => (focused = true)}
    onblur={() => {
      focused = false;
      suggester.clear();
    }}
    {disabled}
  />
  {#if open}
    <ul
      id={listId}
      class="options"
      role="listbox"
      aria-label={m.mail_suggestions({ field: label })}
    >
      {#each options as contact, at (contact.email)}
        <!-- Mouse down, not click: the field must keep focus until the choice lands. -->
        <li
          id={`${listId}-${at}`}
          role="option"
          aria-selected={at === active}
          data-active={at === active}
          onmousedown={(event) => {
            event.preventDefault();
            pick(contact);
          }}
        >
          <span class="name">{displayName(contact)}</span>
          {#if contact.name}<span class="email">{contact.email}</span>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .address {
    position: relative;
    min-width: 0;
  }

  .options {
    position: absolute;
    inset-inline: 0;
    z-index: 2;
    display: grid;
    margin: 4px 0 0;
    padding: 4px;
    max-block-size: 240px;
    overflow-y: auto;
    list-style: none;
    border-radius: var(--radius-menu);
    background: var(--color-raised);
    box-shadow: var(--shadow-popover);
  }

  li {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 2px 8px;
    padding: 6px 8px;
    border-radius: var(--radius-row);
    color: var(--color-text);
    font-size: var(--text-label);
    cursor: default;
  }

  li[data-active="true"],
  li:hover {
    background: var(--color-field-hover);
  }

  .name {
    font-weight: 500;
  }

  .email {
    color: var(--color-muted);
  }

  @media (forced-colors: active) {
    .options {
      border: 1px solid CanvasText;
    }

    li[data-active="true"] {
      outline: 2px solid Highlight;
    }
  }
</style>

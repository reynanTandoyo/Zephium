<!--
  The folders of every account: "All inboxes" first, then each account with
  its special-use folders before the rest, and unread counts. A folder also
  accepts dropped messages.
-->
<script lang="ts">
  import Archive02Icon from "@hugeicons/core-free-icons/Archive02Icon";
  import Delete02Icon from "@hugeicons/core-free-icons/Delete02Icon";
  import Folder01Icon from "@hugeicons/core-free-icons/Folder01Icon";
  import InboxIcon from "@hugeicons/core-free-icons/InboxIcon";
  import SentIcon from "@hugeicons/core-free-icons/SentIcon";
  import SpamIcon from "@hugeicons/core-free-icons/SpamIcon";
  import PencilEdit02Icon from "@hugeicons/core-free-icons/PencilEdit02Icon";
  import Mail01Icon from "@hugeicons/core-free-icons/Mail01Icon";
  import type { IconSvgElement } from "@hugeicons/svelte";
  import type { MailFolderRoleV1, MailFolderV1 } from "$shared/ipc/bindings";
  import * as m from "$shared/i18n/messages";
  import { ALL_INBOXES, type MailStore } from "$domain/mail";
  import Icon from "$shared/ui/Icon";
  import { folderLabel } from "../lib/text";

  let {
    store,
    ondropmessages,
    onselect,
  }: {
    store: MailStore;
    /** Message keys dropped on a folder of an account. */
    ondropmessages: (keys: string[], account: string, folder: string) => void;
    onselect?: () => void;
  } = $props();

  const icons: Record<MailFolderRoleV1, IconSvgElement> = {
    inbox: InboxIcon,
    drafts: PencilEdit02Icon,
    sent: SentIcon,
    archive: Archive02Icon,
    junk: SpamIcon,
    trash: Delete02Icon,
    other: Folder01Icon,
  };

  let dropTarget = $state<string | null>(null);

  const selected = (account: string, path: string) =>
    store.selection.account === account && store.selection.folder === path;

  function acceptsDrop(event: DragEvent) {
    return event.dataTransfer?.types.includes("application/x-zephium-mail") ?? false;
  }

  function drop(event: DragEvent, account: string, folder: MailFolderV1) {
    dropTarget = null;
    const raw = event.dataTransfer?.getData("application/x-zephium-mail");
    if (!raw) return;
    event.preventDefault();
    try {
      const keys: unknown = JSON.parse(raw);
      if (Array.isArray(keys) && keys.every((key) => typeof key === "string")) {
        ondropmessages(keys as string[], account, folder.path);
      }
    } catch {
      // Not ours: ignore a foreign drag.
    }
  }

  const totalUnread = $derived(store.unread);
</script>

<nav class="tree" aria-label={m.mail_folders()}>
  <button
    type="button"
    class="folder"
    aria-current={selected(ALL_INBOXES, "INBOX") ? "page" : undefined}
    onclick={() => {
      void store.select(ALL_INBOXES, "INBOX");
      onselect?.();
    }}
  >
    <Icon icon={Mail01Icon} size={16} />
    <span class="name">{m.mail_all_inboxes()}</span>
    {#if totalUnread > 0}<span
        class="count"
        aria-label={m.mail_folder_unread({ count: totalUnread })}>{totalUnread}</span
      >{/if}
  </button>

  {#each store.accounts as account (account.id)}
    <section class="account" aria-label={account.display_name}>
      <h3>
        <span class="dot" style:background={account.colour} aria-hidden="true"></span>
        <span class="who">{account.display_name}</span>
      </h3>
      {#if account.sign_in_failed}
        <p class="failed" role="status">{m.mail_sign_in_failed()}</p>
      {/if}
      {#if store.folderFailures.includes(account.id)}
        <p class="failed" role="status">{m.mail_folders_failed()}</p>
      {/if}
      {#each store.foldersOf(account.id) as folder (folder.path)}
        <button
          type="button"
          class="folder"
          class:drop={dropTarget === `${account.id}/${folder.path}`}
          aria-current={selected(account.id, folder.path) ? "page" : undefined}
          onclick={() => {
            void store.select(account.id, folder.path);
            onselect?.();
          }}
          ondragover={(event) => {
            if (!acceptsDrop(event)) return;
            event.preventDefault();
            dropTarget = `${account.id}/${folder.path}`;
          }}
          ondragleave={() => (dropTarget = null)}
          ondrop={(event) => drop(event, account.id, folder)}
        >
          <Icon icon={icons[folder.role]} size={16} />
          <span class="name">{folderLabel(folder)}</span>
          {#if folder.unread > 0}<span
              class="count"
              aria-label={m.mail_folder_unread({ count: folder.unread })}>{folder.unread}</span
            >{/if}
        </button>
      {/each}
    </section>
  {/each}
</nav>

<style>
  .tree {
    display: grid;
    align-content: start;
    gap: 2px;
    padding: 8px;
    overflow-y: auto;
    min-inline-size: 0;
  }

  .account {
    display: grid;
    gap: 2px;
    margin-block-start: 12px;
  }

  h3 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    padding: 4px 10px;
    color: var(--color-faint);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    min-inline-size: 0;
  }

  .who {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dot {
    flex: none;
    inline-size: 8px;
    block-size: 8px;
    border-radius: var(--radius-capsule);
  }

  .failed {
    margin: 0;
    padding: 2px 10px;
    color: var(--color-danger);
    font-size: var(--text-label);
  }

  .folder {
    display: flex;
    align-items: center;
    gap: 8px;
    min-inline-size: 0;
    padding: 6px 10px;
    border: 0;
    border-radius: var(--radius-control-compact);
    background: transparent;
    color: var(--color-text);
    font: inherit;
    font-size: var(--text-body);
    text-align: start;
    cursor: pointer;
  }

  .folder:hover {
    background: var(--color-fill-hover);
  }

  .folder[aria-current="page"] {
    background: var(--color-fill-active);
    font-weight: 550;
  }

  .folder.drop {
    background: var(--color-accent-soft);
    outline: 2px solid var(--color-ring);
    outline-offset: -2px;
  }

  .folder:focus-visible {
    outline: 2px solid var(--color-ring);
    outline-offset: -2px;
  }

  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .count {
    color: var(--color-muted);
    font-size: var(--text-label);
    font-variant-numeric: tabular-nums;
  }
</style>

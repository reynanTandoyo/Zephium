<!--
  Mail as a full-window mode: folders, the message list, and the reading
  pane or the message being written. With no account it opens the account
  setup. Rust owns accounts and servers; this page shows what the store
  last read and what each action's answer said.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Archive02Icon from "@hugeicons/core-free-icons/Archive02Icon";
  import ArrowLeft01Icon from "@hugeicons/core-free-icons/ArrowLeft01Icon";
  import Delete02Icon from "@hugeicons/core-free-icons/Delete02Icon";
  import Flag01Icon from "@hugeicons/core-free-icons/Flag01Icon";
  import Forward01Icon from "@hugeicons/core-free-icons/Forward01Icon";
  import FolderTransferIcon from "@hugeicons/core-free-icons/FolderTransferIcon";
  import MailOpen01Icon from "@hugeicons/core-free-icons/MailOpen01Icon";
  import MailReply01Icon from "@hugeicons/core-free-icons/MailReply01Icon";
  import MailReplyAll01Icon from "@hugeicons/core-free-icons/MailReplyAll01Icon";
  import MailRemove01Icon from "@hugeicons/core-free-icons/MailRemove01Icon";
  import PencilEdit02Icon from "@hugeicons/core-free-icons/PencilEdit02Icon";
  import Refresh01Icon from "@hugeicons/core-free-icons/Refresh01Icon";
  import Settings02Icon from "@hugeicons/core-free-icons/Settings02Icon";
  import SpamIcon from "@hugeicons/core-free-icons/SpamIcon";
  import * as m from "$shared/i18n/messages";
  import type { MailAccountV1 } from "$shared/ipc/bindings";
  import { mailtoInbox, MailStore, type MailRow, type MailThread } from "$domain/mail";
  import Button from "$shared/ui/Button";
  import Icon from "$shared/ui/Icon";
  import IconButton from "$shared/ui/IconButton";
  import Menu from "$shared/ui/Menu";
  import type { MenuEntry } from "$shared/ui/Menu";
  import SearchField from "$shared/ui/SearchField";
  import AccountSetupForm from "./AccountSetupForm.svelte";
  import ComposePane from "./ComposePane.svelte";
  import FolderTree from "./FolderTree.svelte";
  import MailSettings from "./MailSettings.svelte";
  import MessageList from "./MessageList.svelte";
  import ReadingPane from "./ReadingPane.svelte";
  import { KEYS, shortcutFor, type MailShortcut } from "../lib/shortcuts";
  import { folderLabel, reasonText, tip } from "../lib/text";

  const store = new MailStore();
  type View = "mail" | "settings" | "add";
  let view = $state<View>("mail");
  let search = $state("");
  let searchField = $state<HTMLInputElement | undefined>();
  let list = $state<ReturnType<typeof MessageList> | undefined>();
  let searchTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    void store.start();
    return () => {
      clearTimeout(searchTimer);
      store.dispose();
    };
  });

  // A clicked mailto: link waits in the inbox until the accounts are read; then
  // it becomes a prefilled message, whatever Mail was showing.
  $effect(() => {
    if (!mailtoInbox.pending || store.accountsState !== "ready" || !store.accounts.length) return;
    const link = mailtoInbox.take();
    if (link && store.startComposeFromLink(link)) view = "mail";
  });

  // A question takes focus on its safe answer, never on the destructive one.
  let keepLink = $state<HTMLButtonElement | undefined>();
  let keepMessages = $state<HTMLButtonElement | undefined>();
  $effect(() => {
    if (store.pendingLink && store.compose) keepLink?.focus();
  });
  $effect(() => {
    if (store.permanentDelete) keepMessages?.focus();
  });

  // The message that held a waiting link was closed or sent: open the link now.
  $effect(() => {
    if (store.pendingLink && !store.compose) void store.acceptPendingLink(false);
  });

  const ready = $derived(store.open?.state === "ready" ? store.open : null);
  const row = $derived(ready?.row ?? null);
  const showSetup = $derived(
    view === "add" || (store.accountsState === "ready" && store.accounts.length === 0),
  );
  const detailShown = $derived(store.compose !== null || store.open !== null);
  const title = $derived.by(() => {
    const current = store.current;
    if (current.combined) return m.mail_all_inboxes();
    return current.folder ? folderLabel(current.folder) : "";
  });

  const moveEntries = $derived.by((): MenuEntry[] => {
    if (!row) return [];
    return store
      .foldersOf(row.account)
      .filter((folder) => folder.path !== row.folder)
      .map((folder) => ({ kind: "item", id: folder.path, label: folderLabel(folder) }));
  });

  function onsearch(value: string) {
    search = value;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void store.search(value), 300);
  }

  function clearSearch() {
    clearTimeout(searchTimer);
    search = "";
    void store.search("");
  }

  async function added(account: MailAccountV1) {
    await store.accountAdded(account);
    view = "mail";
  }

  function openThread(thread: MailThread) {
    void store.openRow(thread.latest);
    store.closeCompose();
  }

  /** Opens the neighbour of the open conversation, or the first one. */
  function step(delta: 1 | -1) {
    const threads = store.threads;
    if (!threads.length) return;
    const at = threads.findIndex((t) => t.rows.some((r) => r.key === store.open?.row.key));
    const next = threads[at < 0 ? 0 : Math.min(threads.length - 1, Math.max(0, at + delta))];
    if (!next) return;
    openThread(next);
    list?.reveal(next);
  }

  function act(action: MailShortcut) {
    switch (action) {
      case "next":
        return step(1);
      case "previous":
        return step(-1);
      case "compose":
        return void store.startCompose("new");
      case "search":
        return searchField?.focus();
      case "sync":
        return void store.sync();
    }
    if (!row) return;
    switch (action) {
      case "reply":
        return void store.startCompose("reply");
      case "reply_all":
        return void store.startCompose("reply_all");
      case "forward":
        return void store.startCompose("forward");
      case "toggle_read":
        return void store.setSeen([row], !row.message.flags.seen);
      case "toggle_flag":
        return void store.setFlagged([row], !row.message.flags.flagged);
      case "archive":
        return void store.moveToRole([row], "archive");
      case "junk":
        return void store.moveToRole([row], "junk");
      case "delete":
        return void store.remove([row]);
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (view !== "mail" || store.compose) return;
    const action = shortcutFor(event);
    if (!action) return;
    event.preventDefault();
    act(action);
  }

  function dropped(keys: string[], account: string, folder: string) {
    const wanted = new Set(keys);
    const rows = store.rows.filter((r) => wanted.has(r.key) && r.account === account);
    if (rows.length) void store.move(rows, folder);
  }

  const selectedRows = (): MailRow[] => (row ? [row] : []);
</script>

<!-- Keys work with focus anywhere on the page, including after the focused row left. -->
<svelte:window {onkeydown} />

<section class="page" aria-label={m.mode_mail()}>
  {#if store.accountsState === "idle" || store.accountsState === "loading"}
    <p class="center" role="status">{m.mail_loading()}</p>
  {:else if store.accountsState === "failed"}
    <div class="center" role="alert">
      <p>{m.mail_page_failed()}</p>
      <Button onclick={() => void store.loadAccounts()}>{m.mail_retry()}</Button>
    </div>
  {:else if showSetup}
    <div class="scroll">
      <AccountSetupForm
        colourSeed={store.accounts.length}
        onadded={(account) => void added(account)}
        oncancel={store.accounts.length ? () => (view = "mail") : undefined}
      />
    </div>
  {:else if view === "settings"}
    <div class="scroll">
      <MailSettings {store} onadd={() => (view = "add")} onback={() => (view = "mail")} />
    </div>
  {:else}
    <div class="mail" data-detail={detailShown}>
      <aside class="side">
        <div class="compose-button">
          <Button
            variant="primary"
            title={tip(m.mail_action_new(), KEYS.compose)}
            onclick={() => store.startCompose("new")}
          >
            <Icon icon={PencilEdit02Icon} size={16} />
            {m.mail_action_new()}
          </Button>
        </div>
        <FolderTree {store} ondropmessages={dropped} />
        <div class="side-foot">
          <IconButton
            icon={Refresh01Icon}
            label={tip(m.mail_sync(), KEYS.sync)}
            pending={store.syncing}
            onclick={() => void store.sync()}
          />
          <IconButton
            icon={Settings02Icon}
            label={m.mail_settings_open()}
            onclick={() => (view = "settings")}
          />
        </div>
      </aside>

      <div class="listcol">
        <div class="list-head">
          <h2>{title}</h2>
          {#if store.offline}
            <p class="offline" role="status" title={m.mail_offline_hint()}>
              {m.mail_offline_badge()}
            </p>
          {/if}
          <SearchField
            label={m.mail_search()}
            placeholder={m.mail_search()}
            size="page"
            value={search}
            bind:ref={searchField}
            oninput={onsearch}
            onsubmit={(value) => {
              clearTimeout(searchTimer);
              void store.search(value);
            }}
          />
          {#if search}
            <Button variant="ghost" size="compact" onclick={clearSearch}
              >{m.mail_search_clear()}</Button
            >
          {/if}
        </div>
        <MessageList {store} bind:this={list} onopen={openThread} />
      </div>

      <div class="detail">
        <div class="bar" role="toolbar" aria-label={m.mail_actions()}>
          <Button
            class="back"
            variant="ghost"
            size="compact"
            onclick={() => {
              store.close();
              store.closeCompose();
            }}
          >
            <Icon icon={ArrowLeft01Icon} size={16} />
            {m.mail_settings_back()}
          </Button>
          <IconButton
            icon={MailReply01Icon}
            label={tip(m.mail_action_reply(), KEYS.reply)}
            disabled={!row}
            onclick={() => store.startCompose("reply")}
          />
          <IconButton
            icon={MailReplyAll01Icon}
            label={tip(m.mail_action_reply_all(), KEYS.reply_all)}
            disabled={!row}
            onclick={() => store.startCompose("reply_all")}
          />
          <IconButton
            icon={Forward01Icon}
            label={tip(m.mail_action_forward(), KEYS.forward)}
            disabled={!row}
            onclick={() => store.startCompose("forward")}
          />
          <span class="gap"></span>
          <IconButton
            icon={row?.message.flags.seen === false ? MailOpen01Icon : MailRemove01Icon}
            label={tip(
              row?.message.flags.seen === false
                ? m.mail_action_mark_read()
                : m.mail_action_mark_unread(),
              KEYS.toggle_read,
            )}
            disabled={!row || store.isPending(row.key)}
            onclick={() => row && void store.setSeen([row], !row.message.flags.seen)}
          />
          <IconButton
            icon={Flag01Icon}
            label={tip(
              row?.message.flags.flagged ? m.mail_action_unflag() : m.mail_action_flag(),
              KEYS.toggle_flag,
            )}
            active={row?.message.flags.flagged ?? false}
            disabled={!row || store.isPending(row.key)}
            onclick={() => row && void store.setFlagged([row], !row.message.flags.flagged)}
          />
          <IconButton
            icon={Archive02Icon}
            label={tip(m.mail_action_archive(), KEYS.archive)}
            disabled={!row}
            onclick={() => void store.moveToRole(selectedRows(), "archive")}
          />
          <Menu
            label={m.mail_action_move()}
            entries={[{ kind: "heading", label: m.mail_move_heading() }, ...moveEntries]}
            align="end"
            triggerClass="mail-move"
            onselect={(path) => void store.move(selectedRows(), path)}
          >
            {#snippet trigger()}<Icon icon={FolderTransferIcon} size={16} />{/snippet}
          </Menu>
          <IconButton
            icon={SpamIcon}
            label={tip(m.mail_action_junk(), KEYS.junk)}
            disabled={!row}
            onclick={() => void store.moveToRole(selectedRows(), "junk")}
          />
          <IconButton
            icon={Delete02Icon}
            label={tip(m.mail_action_delete(), KEYS.delete)}
            disabled={!row}
            onclick={() => void store.remove(selectedRows())}
          />
        </div>

        {#if store.notice}
          <div class="notice" role="alert">
            <span>{m.mail_notice_failed({ reason: reasonText(store.notice.error) })}</span>
            <Button variant="ghost" size="compact" onclick={() => store.dismissNotice()}
              >{m.mail_dismiss()}</Button
            >
          </div>
        {/if}

        {#if store.pendingLink && store.compose}
          <div class="confirm" role="alertdialog" aria-labelledby="mail-link-title">
            <p id="mail-link-title">{m.mail_link_compose_title()}</p>
            <div class="actions">
              <Button
                variant="primary"
                disabled={store.compose.busy}
                onclick={() => void store.acceptPendingLink(true)}
                >{m.mail_link_compose_save()}</Button
              >
              <Button
                variant="danger"
                disabled={store.compose.busy}
                onclick={() => void store.acceptPendingLink(false)}
                >{m.mail_link_compose_discard()}</Button
              >
              <Button bind:ref={keepLink} onclick={() => store.dismissPendingLink()}
                >{m.mail_link_compose_keep()}</Button
              >
            </div>
          </div>
        {/if}

        {#if store.permanentDelete}
          <div class="confirm" role="alertdialog" aria-labelledby="mail-delete-title">
            <p id="mail-delete-title">{m.mail_delete_forever_title()}</p>
            <div class="actions">
              <Button variant="danger" onclick={() => void store.confirmPermanentDelete()}
                >{m.mail_delete_forever_confirm()}</Button
              >
              <Button bind:ref={keepMessages} onclick={() => store.cancelPermanentDelete()}
                >{m.mail_delete_forever_cancel()}</Button
              >
            </div>
          </div>
        {/if}

        {#if store.compose}
          <ComposePane {store} compose={store.compose} onclose={() => store.closeCompose()} />
        {:else}
          <ReadingPane {store} />
        {/if}
      </div>
    </div>
  {/if}
</section>

<style>
  .page {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
    color: var(--color-text);
    container-type: inline-size;
  }

  .center {
    display: grid;
    justify-items: center;
    align-content: center;
    gap: 12px;
    flex: 1;
    margin: 0;
    color: var(--color-muted);
    font-size: var(--text-body);
  }

  .scroll {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
  }

  .mail {
    display: grid;
    grid-template-columns: 220px minmax(280px, 380px) minmax(0, 1fr);
    flex: 1;
    min-block-size: 0;
  }

  .side {
    display: flex;
    flex-direction: column;
    min-block-size: 0;
    border-inline-end: 1px solid var(--color-border);
  }

  .side :global(.tree) {
    flex: 1;
    min-block-size: 0;
  }

  .compose-button {
    padding: 12px 8px 4px;
  }

  .compose-button :global(button) {
    inline-size: 100%;
  }

  .side-foot {
    display: flex;
    justify-content: space-between;
    padding: 8px;
    border-block-start: 1px solid var(--color-border);
  }

  .listcol {
    display: flex;
    flex-direction: column;
    min-block-size: 0;
    min-inline-size: 0;
  }

  .listcol :global(.list) {
    flex: 1;
  }

  .list-head {
    display: grid;
    gap: 8px;
    padding: 12px 12px 8px;
    border-inline-end: 1px solid var(--color-border);
  }

  h2 {
    margin: 0;
    font-size: var(--text-page-title);
    font-weight: 600;
  }

  .offline {
    justify-self: start;
    margin: 0;
    padding: 2px 8px;
    border-radius: var(--radius-capsule);
    background: var(--color-fill);
    color: var(--color-muted);
    font-size: var(--text-label);
    font-weight: 500;
  }

  .detail {
    display: flex;
    flex-direction: column;
    min-block-size: 0;
    min-inline-size: 0;
  }

  .detail > :global(:last-child) {
    flex: 1;
  }

  .bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 2px;
    padding: 8px 12px;
    border-block-end: 1px solid var(--color-border);
  }

  .gap {
    flex: 1;
  }

  .bar :global(.back) {
    display: none;
  }

  .bar :global(.mail-move) {
    display: inline-grid;
    place-items: center;
    inline-size: 28px;
    block-size: 28px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-control-compact);
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  .bar :global(.mail-move:hover) {
    background: var(--color-fill-hover);
  }

  .bar :global(.mail-move:focus-visible) {
    outline: 2px solid var(--color-ring);
  }

  .notice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 16px;
    border-block-end: 1px solid var(--color-border);
    background: var(--color-fill);
    color: var(--color-danger);
    font-size: var(--text-label);
  }

  .confirm {
    display: grid;
    gap: 10px;
    padding: 10px 16px;
    border-block-end: 1px solid var(--color-border);
    background: var(--color-raised);
  }

  .confirm p {
    margin: 0;
    font-weight: 550;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  @container (max-width: 900px) {
    .mail {
      grid-template-columns: 180px minmax(0, 1fr);
    }

    .mail[data-detail="true"] .listcol,
    .mail[data-detail="false"] .detail {
      display: none;
    }

    .mail[data-detail="true"] .side {
      display: none;
    }

    .mail[data-detail="true"] {
      grid-template-columns: minmax(0, 1fr);
    }

    .bar :global(.back) {
      display: inline-flex;
    }
  }
</style>

<!--
  The message list: one row per conversation, newest first, windowed so a
  long folder stays cheap. Rows are fixed height, which keeps the window a
  prefix sum. A row can be dragged to a folder.
-->
<script lang="ts">
  import Attachment01Icon from "@hugeicons/core-free-icons/Attachment01Icon";
  import Flag01Icon from "@hugeicons/core-free-icons/Flag01Icon";
  import * as m from "$shared/i18n/messages";
  import { ALL_INBOXES, displayName, type MailStore, type MailThread } from "$domain/mail";
  import { createVirtualWindow } from "$shared/lib/virtual-window.svelte";
  import Button from "$shared/ui/Button";
  import Icon from "$shared/ui/Icon";
  import { formatWhen, reasonText } from "../lib/text";

  let {
    store,
    onopen,
  }: {
    store: MailStore;
    onopen: (thread: MailThread) => void;
  } = $props();

  const ROW_HEIGHT = 68;
  const threads = $derived(store.threads);
  const virtual = createVirtualWindow({ heights: () => threads.map(() => ROW_HEIGHT) });
  const slice = $derived(virtual.window);
  const visible = $derived(threads.slice(slice.first, slice.last));
  const openKey = $derived(store.open?.row.key ?? null);
  const combined = $derived(store.selection.account === ALL_INBOXES);

  const isOpen = (thread: MailThread) => thread.rows.some((row) => row.key === openKey);

  /** Keeps the keyboard-selected row visible, windowed or not. */
  export function reveal(thread: MailThread) {
    virtual.scrollToIndex(threads.indexOf(thread));
  }

  function nearEnd(event: Event) {
    const element = event.currentTarget as HTMLElement;
    if (element.scrollHeight - element.scrollTop - element.clientHeight < ROW_HEIGHT * 6) {
      void store.loadMore();
    }
  }

  function dragStart(event: DragEvent, thread: MailThread) {
    if (!event.dataTransfer) return;
    event.dataTransfer.setData(
      "application/x-zephium-mail",
      JSON.stringify(thread.rows.map((row) => row.key)),
    );
    event.dataTransfer.effectAllowed = "move";
  }

  const sender = (thread: MailThread) =>
    displayName(thread.latest.message.from[0]) || m.mail_no_sender();
  const colourOf = (account: string) => store.account(account)?.colour ?? "transparent";
</script>

<div class="list" role="region" aria-label={m.mail_list()}>
  {#if store.fromCache}
    <p class="banner" role="status">{m.mail_offline_cache()}</p>
  {/if}
  {#if store.listState === "failed"}
    <div class="state" role="alert">
      <p>{m.mail_list_failed({ reason: reasonText(store.listError ?? "internal") })}</p>
      <Button onclick={() => void store.loadList()}>{m.mail_retry()}</Button>
    </div>
  {:else if store.listState === "loading" && !threads.length}
    <p class="state" role="status">{m.mail_loading()}</p>
  {:else if !threads.length}
    <p class="state" role="status">
      {store.searching ? m.mail_search_empty() : m.mail_empty_folder()}
    </p>
  {:else}
    <div class="scroll" use:virtual.attach onscroll={nearEnd}>
      <ul
        role="listbox"
        aria-label={m.mail_list()}
        style:padding-block-start={`${slice.before}px`}
        style:padding-block-end={`${slice.after}px`}
      >
        {#each visible as thread, offset (thread.key)}
          {@const message = thread.latest.message}
          {@const unread = thread.unread > 0}
          <li
            role="option"
            aria-selected={isOpen(thread)}
            data-virtual-index={slice.first + offset}
            style:block-size={`${ROW_HEIGHT}px`}
          >
            <button
              type="button"
              class="row"
              class:unread
              class:pending={thread.rows.some((row) => store.isPending(row.key))}
              aria-busy={thread.rows.some((row) => store.isPending(row.key)) || undefined}
              draggable="true"
              data-thread={thread.key}
              onclick={() => onopen(thread)}
              ondragstart={(event) => dragStart(event, thread)}
            >
              <span
                class="rail"
                style:background={combined ? colourOf(thread.latest.account) : "transparent"}
                aria-hidden="true"
              ></span>
              <span class="top">
                <span class="from">{sender(thread)}</span>
                {#if thread.rows.length > 1}
                  <span class="thread" title={m.mail_thread_count({ count: thread.rows.length })}
                    >{thread.rows.length}</span
                  >
                {/if}
                {#if unread}<span class="mark" role="img" aria-label={m.mail_unread()}></span>{/if}
                <time class="when">{formatWhen(message.date)}</time>
              </span>
              <span class="subject">{thread.subject || m.mail_no_subject()}</span>
              <span class="bottom">
                <span class="snippet">{message.snippet}</span>
                {#if thread.attachments}<span
                    class="icon"
                    role="img"
                    aria-label={m.mail_has_attachment()}
                    ><Icon icon={Attachment01Icon} size={14} /></span
                  >{/if}
                {#if thread.flagged}<span class="icon flag" role="img" aria-label={m.mail_flagged()}
                    ><Icon icon={Flag01Icon} size={14} /></span
                  >{/if}
              </span>
            </button>
          </li>
        {/each}
      </ul>
      {#if store.loadingMore}<p class="more" role="status">{m.mail_loading()}</p>
      {:else if store.hasMore && !store.searching}
        <div class="more">
          <Button variant="ghost" onclick={() => void store.loadMore()}>{m.mail_load_more()}</Button
          >
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .list {
    display: flex;
    flex-direction: column;
    min-block-size: 0;
    min-inline-size: 0;
    border-inline-end: 1px solid var(--color-border);
  }

  .scroll {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    box-sizing: border-box;
  }

  li {
    box-sizing: border-box;
    padding: 2px 6px;
  }

  .row {
    position: relative;
    display: grid;
    grid-template-rows: auto auto 1fr;
    gap: 1px;
    inline-size: 100%;
    block-size: 100%;
    box-sizing: border-box;
    padding: 6px 10px 6px 14px;
    border: 0;
    border-radius: var(--radius-row);
    background: transparent;
    color: var(--color-text);
    font: inherit;
    text-align: start;
    cursor: pointer;
    min-inline-size: 0;
    overflow: hidden;
  }

  .row:hover {
    background: var(--color-fill-hover);
  }

  .row:focus-visible {
    outline: 2px solid var(--color-ring);
    outline-offset: -2px;
  }

  li[aria-selected="true"] .row {
    background: var(--color-fill-active);
  }

  .row.pending {
    opacity: 0.6;
  }

  .rail {
    position: absolute;
    inset-block: 8px;
    inset-inline-start: 4px;
    inline-size: 3px;
    border-radius: var(--radius-capsule);
  }

  .top,
  .bottom {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
  }

  .from {
    flex: 1;
    overflow: hidden;
    font-size: var(--text-body);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .thread {
    color: var(--color-muted);
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
  }

  .mark {
    inline-size: 8px;
    block-size: 8px;
    border-radius: var(--radius-capsule);
    background: var(--color-accent);
  }

  .when {
    color: var(--color-faint);
    font-size: var(--text-caption);
    white-space: nowrap;
  }

  .subject {
    overflow: hidden;
    font-size: var(--text-body);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .unread .from,
  .unread .subject {
    font-weight: 650;
  }

  .snippet {
    flex: 1;
    overflow: hidden;
    color: var(--color-muted);
    font-size: var(--text-label);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .icon {
    display: inline-flex;
    color: var(--color-muted);
  }

  .flag {
    color: var(--color-warning);
  }

  .state,
  .banner,
  .more {
    margin: 0;
    padding: 16px;
    color: var(--color-muted);
    font-size: var(--text-body);
    text-align: center;
  }

  .state {
    display: grid;
    justify-items: center;
    gap: 12px;
  }

  .banner {
    padding: 6px 12px;
    border-block-end: 1px solid var(--color-border);
    background: var(--color-fill);
    font-size: var(--text-label);
  }
</style>

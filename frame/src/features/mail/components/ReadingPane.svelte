<!--
  The reading pane. Plain text is drawn as text. HTML is sanitised first and
  then shown in a sandboxed frame with no scripts, no same-origin access and a
  CSP that allows no network, so it cannot run code or call out. Remote images
  wait for "Load images", per message: the backend fetches the https ones and
  they are shown as data URLs, so the frame still never touches the network.
  Links never navigate here (the frame has no script to report a click): they
  are listed below the message, and their real target is shown and opened as a
  browser tab on confirmation.
-->
<script lang="ts">
  import Attachment01Icon from "@hugeicons/core-free-icons/Attachment01Icon";
  import Link01Icon from "@hugeicons/core-free-icons/Link01Icon";
  import { commands } from "$shared/ipc/bindings";
  import * as m from "$shared/i18n/messages";
  import {
    displayName,
    formatAddresses,
    inlineImageUrl,
    linkify,
    mailFrameDocument,
    sanitizeMailHtml,
    type MailStore,
  } from "$domain/mail";
  import Button from "$shared/ui/Button";
  import Icon from "$shared/ui/Icon";
  import { formatFull, formatSize, reasonText } from "../lib/text";

  let { store }: { store: MailStore } = $props();

  const open = $derived(store.open);
  const body = $derived(open?.body ?? null);
  const header = $derived(body?.header ?? open?.row.message ?? null);

  /** A link the person clicked, waiting to be opened or dismissed. */
  let target = $state<string | null>(null);
  let saved = $state<Record<number, "saved" | "failed">>({});
  let linksOpen = $state(false);

  $effect(() => {
    void open?.row.key;
    target = null;
    saved = {};
    linksOpen = false;
  });

  const remote = $derived(open?.remote ?? null);
  const inline = $derived(
    new Map(
      (body?.inline_images ?? []).flatMap((image) => {
        const url = inlineImageUrl(image.content_type, image.data_base64);
        return url ? [[image.content_id.replace(/^<|>$/gu, ""), url] as const] : [];
      }),
    ),
  );
  const rendered = $derived(
    body?.html
      ? sanitizeMailHtml(body.html, {
          inline,
          remoteImages: new Map(Object.entries(remote?.images ?? {})),
        })
      : null,
  );
  const frame = $derived(rendered ? mailFrameDocument(rendered.html) : null);
  /** Remote images that are still not shown (not asked for, refused, or not https). */
  const imagesLeft = $derived(rendered ? rendered.remoteImages - rendered.remoteLoaded : 0);
  const segments = $derived(!rendered && body?.text ? linkify(body.text) : null);
  const links = $derived(
    rendered
      ? [...new Set(rendered.links)]
      : (segments?.flatMap((s) => (s.url ? [s.url] : [])) ?? []),
  );
  const attachments = $derived((body?.attachments ?? []).filter((a) => !a.inline || !a.content_id));
  const thread = $derived(
    open ? store.threads.find((t) => t.rows.some((row) => row.key === open.row.key)) : undefined,
  );

  function loadImages() {
    if (rendered) void store.loadRemoteImages(rendered.remoteUrls);
  }

  /** Opens the shown target as a browser tab; the pane itself never navigates. */
  function openTarget() {
    // A mail address opens Mail's own compose, never another application.
    if (target && /^mailto:/iu.test(target)) void store.composeFromLink(target);
    else if (target) void commands.browserOpenUrl(target, true);
    target = null;
  }

  async function save(part: number) {
    if (!open) return;
    const outcome = await store.saveAttachment(open.row, part);
    if (outcome === "saved") saved = { ...saved, [part]: "saved" };
    else if (outcome === null) saved = { ...saved, [part]: "failed" };
  }
</script>

<article class="pane" aria-label={header?.subject || m.mail_no_subject()}>
  {#if !open}
    <p class="empty">{m.mail_select_message()}</p>
  {:else if open.state === "loading"}
    <p class="empty" role="status">{m.mail_loading()}</p>
  {:else if open.state === "failed"}
    <div class="empty" role="alert">
      <p>{m.mail_open_failed({ reason: reasonText(open.error ?? "internal") })}</p>
      <Button onclick={() => void store.openRow(open.row)}>{m.mail_retry()}</Button>
    </div>
  {:else if header && body}
    <header>
      <h2>{header.subject || m.mail_no_subject()}</h2>
      <dl>
        <dt>{m.mail_from()}</dt>
        <dd>{formatAddresses(header.from) || m.mail_no_sender()}</dd>
        {#if header.to.length}<dt>{m.mail_to()}</dt>
          <dd>{formatAddresses(header.to)}</dd>{/if}
        {#if header.cc.length}<dt>{m.mail_cc()}</dt>
          <dd>{formatAddresses(header.cc)}</dd>{/if}
        <dt>{m.mail_date()}</dt>
        <dd>{formatFull(header.date)}</dd>
      </dl>
      {#if thread && thread.rows.length > 1}
        <nav class="conversation" aria-label={m.mail_thread_count({ count: thread.rows.length })}>
          {#each thread.rows as row (row.key)}
            <button
              type="button"
              aria-current={row.key === open.row.key ? "true" : undefined}
              onclick={() => void store.openRow(row)}
            >
              {displayName(row.message.from[0])}
            </button>
          {/each}
        </nav>
      {/if}
    </header>

    {#if rendered && imagesLeft > 0}
      <div class="banner" role="status">
        {#if remote?.state === "failed"}
          <span>{m.mail_images_failed({ reason: reasonText(remote.error ?? "internal") })}</span>
        {:else if remote?.state === "ready"}
          <span>{m.mail_images_partial()}</span>
        {:else}
          <span>{m.mail_images_blocked()}</span>
        {/if}
        {#if remote?.state !== "ready" && rendered.remoteUrls.length > 0}
          <Button size="compact" pending={remote?.state === "loading"} onclick={loadImages}>
            {m.mail_load_images()}
          </Button>
        {/if}
      </div>
    {/if}
    {#if body.truncated}<p class="banner" role="status">{m.mail_truncated()}</p>{/if}

    {#if target}
      <div class="target" role="alertdialog" aria-label={m.mail_link_target()}>
        <span class="label">{m.mail_link_target()}</span>
        <code class="url">{target}</code>
        <Button variant="primary" size="compact" onclick={openTarget}>{m.mail_link_open()}</Button>
        <Button variant="ghost" size="compact" onclick={() => (target = null)}
          >{m.mail_settings_cancel()}</Button
        >
      </div>
    {/if}

    {#if frame}
      <iframe
        class="html"
        title={m.mail_body_frame()}
        sandbox=""
        referrerpolicy="no-referrer"
        srcdoc={frame}
      ></iframe>
    {:else if segments}
      <div class="text">
        {#each segments as part, at (at)}{#if part.url}<button
              type="button"
              class="link"
              title={part.url}
              onclick={() => (target = part.url)}>{part.text}</button
            >{:else}{part.text}{/if}{/each}
      </div>
    {:else}
      <p class="empty">{m.mail_no_body()}</p>
    {/if}

    {#if links.length && rendered}
      <details class="links" bind:open={linksOpen}>
        <summary
          ><Icon icon={Link01Icon} size={14} /> {m.mail_links_title()} ({links.length})</summary
        >
        <ul>
          {#each links as url (url)}
            <li>
              <button type="button" class="link" title={url} onclick={() => (target = url)}
                >{url}</button
              >
            </li>
          {/each}
        </ul>
      </details>
    {/if}

    {#if attachments.length}
      <section class="attachments" aria-label={m.mail_attachments()}>
        <h3>{m.mail_attachments()}</h3>
        <ul>
          {#each attachments as attachment (attachment.part)}
            <li>
              <Icon icon={Attachment01Icon} size={14} />
              <span class="file">{attachment.filename || m.mail_attachment_unnamed()}</span>
              <span class="size">{formatSize(attachment.size)}</span>
              <Button
                size="compact"
                aria-label={m.mail_attachment_save({ name: attachment.filename })}
                onclick={() => void save(attachment.part)}>{m.mail_save()}</Button
              >
              {#if saved[attachment.part] === "saved"}<span class="ok" role="status"
                  >{m.mail_attachment_saved()}</span
                >{/if}
              {#if saved[attachment.part] === "failed"}<span class="bad" role="status"
                  >{m.mail_attachment_failed()}</span
                >{/if}
            </li>
          {/each}
        </ul>
      </section>
    {/if}
  {/if}
</article>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    min-block-size: 0;
    min-inline-size: 0;
    overflow-y: auto;
    color: var(--color-text);
  }

  .empty {
    display: grid;
    place-items: center;
    align-content: center;
    gap: 12px;
    flex: 1;
    margin: 0;
    padding: 24px;
    color: var(--color-muted);
    font-size: var(--text-body);
    text-align: center;
  }

  header {
    display: grid;
    gap: 8px;
    padding: 16px 20px 12px;
    border-block-end: 1px solid var(--color-border);
  }

  h2 {
    margin: 0;
    font-size: var(--text-object-title);
    font-weight: 600;
    letter-spacing: -0.01em;
    overflow-wrap: anywhere;
  }

  dl {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 2px 12px;
    margin: 0;
    font-size: var(--text-label);
  }

  dt {
    color: var(--color-faint);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .conversation {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .conversation button {
    padding: 2px 8px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-capsule);
    background: transparent;
    color: var(--color-muted);
    font: inherit;
    font-size: var(--text-label);
    cursor: pointer;
  }

  .conversation button[aria-current="true"] {
    background: var(--color-fill-active);
    color: var(--color-text);
  }

  .banner,
  .target {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 12px;
    margin: 0;
    padding: 8px 20px;
    border-block-end: 1px solid var(--color-border);
    background: var(--color-fill);
    font-size: var(--text-label);
  }

  .banner span {
    flex: 1;
  }

  .target .label {
    color: var(--color-muted);
  }

  .url {
    flex: 1 1 100%;
    order: 3;
    padding: 4px 8px;
    border-radius: var(--radius-inset);
    background: var(--color-field);
    font-family: var(--font-mono);
    font-size: var(--text-label);
    overflow-wrap: anywhere;
    user-select: all;
  }

  .html {
    flex: 1;
    min-block-size: 320px;
    inline-size: 100%;
    border: 0;
  }

  .text {
    margin: 0;
    padding: 16px 20px;
    font-family: var(--font-mono);
    font-size: var(--text-body);
    line-height: 1.55;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-text);
    font: inherit;
    text-align: start;
    text-decoration: underline;
    overflow-wrap: anywhere;
    cursor: pointer;
  }

  .conversation button:focus-visible,
  .link:focus-visible {
    outline: 2px solid var(--color-ring);
    outline-offset: 1px;
  }

  .links,
  .attachments {
    padding: 8px 20px;
    border-block-start: 1px solid var(--color-border);
    font-size: var(--text-label);
  }

  .links summary {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--color-muted);
    cursor: pointer;
  }

  ul {
    display: grid;
    gap: 4px;
    margin: 6px 0 0;
    padding: 0;
    list-style: none;
  }

  h3 {
    margin: 0;
    color: var(--color-faint);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .attachments li {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .file {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size {
    color: var(--color-faint);
    font-variant-numeric: tabular-nums;
  }

  .ok {
    color: var(--color-muted);
  }

  .bad {
    color: var(--color-danger);
  }
</style>

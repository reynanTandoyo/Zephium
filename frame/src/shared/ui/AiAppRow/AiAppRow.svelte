<script module lang="ts">
  /** What the row shows of an AI app on this PC; mirrors the native `WorkAiAppV1`. */
  export type AiAppView = {
    state: "missing" | "sign_in_needed" | "ready" | "unknown";
    version: string | null;
    install: { command: string | null; url: string };
  };
</script>

<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import Button from "$shared/ui/Button";
  import ProviderMark from "$shared/ui/ProviderMark";

  let {
    provider,
    name,
    app,
    busy = false,
    failed = false,
    oninstall,
    onsignin,
  }: {
    provider: "claude_code" | "antigravity" | "open_code";
    name: string;
    /** The app as last scanned, or `null` while it is being checked. */
    app: AiAppView | null;
    /** Its install or sign-in is being started. */
    busy?: boolean;
    /** Its last install or sign-in could not be started. */
    failed?: boolean;
    oninstall: () => void;
    onsignin: () => void;
  } = $props();

  const status = $derived.by(() => {
    switch (app?.state) {
      case undefined:
        return { tone: "quiet", text: m.connections_checking() };
      case "ready":
        return { tone: "good", text: m.ai_app_ready() };
      case "sign_in_needed":
        return { tone: "attention", text: m.ai_app_signin_needed() };
      case "missing":
        return { tone: "quiet", text: m.ai_app_missing() };
      default:
        return { tone: "attention", text: m.ai_app_unknown() };
    }
  });

  let confirming = $state(false);
  const uid = $props.id();

  function confirm() {
    confirming = false;
    oninstall();
  }

  function escape(event: KeyboardEvent) {
    if (event.key !== "Escape" || !confirming) return;
    event.preventDefault();
    event.stopPropagation();
    confirming = false;
  }
</script>

<div class="app-row" data-missing={app?.state === "missing"} onkeydown={escape} role="presentation">
  <div class="head">
    <span class="tile" aria-hidden="true"><ProviderMark {provider} size={20} /></span>
    <div class="copy">
      <h3>{name}</h3>
      <p>
        <span class="status" data-tone={status.tone}
          ><span class="dot" aria-hidden="true"></span>{status.text}</span
        >
      </p>
    </div>
    {#if app?.state === "ready" && app.version}<span class="aside version">{app.version}</span>{/if}
    {#if app?.state === "sign_in_needed"}<Button size="compact" pending={busy} onclick={onsignin}
        >{m.ai_app_sign_in()}</Button
      >{:else if app?.state === "missing"}<Button
        size="compact"
        aria-expanded={confirming}
        aria-controls={`${uid}-confirm`}
        disabled={busy}
        onclick={() => (confirming = !confirming)}>{m.connections_install()}</Button
      >{/if}
  </div>
  {#if confirming && app}
    <div
      class="confirm"
      id={`${uid}-confirm`}
      role="group"
      aria-label={m.ai_app_install_title({ app: name })}
    >
      <p class="title">{m.ai_app_install_title({ app: name })}</p>
      {#if app.install.command}
        <p>{m.ai_app_install_command()}</p>
        <code>{app.install.command}</code>
        <p>{m.ai_app_install_after()}</p>
      {:else}
        <p>{m.ai_app_install_page({ app: name })}</p>
      {/if}
      <div class="buttons">
        <Button size="compact" variant="ghost" onclick={() => (confirming = false)}
          >{m.ai_key_cancel()}</Button
        >
        <Button size="compact" variant="primary" onclick={confirm}>{m.connections_install()}</Button
        >
      </div>
    </div>
  {/if}
  {#if failed}<p class="note" role="alert">{m.ai_app_action_failed()}</p>{/if}
</div>

<style>
  /* Row metrics match SettingsRow and the connections rows. */
  .app-row {
    position: relative;
    box-sizing: border-box;
    padding: 12px 18px 12px 14px;
  }

  :global(.app-row) + .app-row::before {
    content: "";
    position: absolute;
    inset-inline: 58px 0;
    top: 0;
    height: 1px;
    background: var(--color-border);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: calc(var(--row-page) - 24px);
  }

  .tile {
    display: grid;
    flex: none;
    inline-size: 32px;
    block-size: 32px;
    border-radius: var(--radius-control);
    background: var(--color-fill);
    color: var(--color-text);
    place-items: center;
  }

  .copy {
    flex: 1;
    min-width: 0;
  }

  h3 {
    margin: 0;
    color: var(--color-text);
    font-size: var(--text-page-title);
    font-weight: 500;
    line-height: 19px;
    letter-spacing: -0.008em;
  }

  [data-missing="true"] h3 {
    color: var(--color-muted);
  }

  p {
    margin: 2px 0 0;
    color: var(--color-muted);
    font-size: var(--text-label);
    line-height: 1.5;
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .dot {
    flex: none;
    inline-size: 6px;
    block-size: 6px;
    border-radius: var(--radius-capsule);
    background: var(--color-faint);
  }

  .status[data-tone="good"] .dot {
    background: var(--color-success);
  }

  .status[data-tone="attention"] .dot {
    background: var(--color-warning);
  }

  .aside {
    flex: none;
    color: var(--color-faint);
    font-size: var(--text-label);
  }

  .version {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
  }

  .confirm {
    display: grid;
    gap: 8px;
    margin: 12px 0 2px 44px;
    animation: confirm-in var(--motion-base) var(--ease-out);
  }

  .confirm p {
    margin: 0;
  }

  .confirm .title {
    color: var(--color-text);
    font-weight: 500;
  }

  code {
    padding: 8px 10px;
    overflow-wrap: anywhere;
    user-select: text;
    border-radius: var(--radius-inset);
    background: var(--color-fill);
    color: var(--color-text);
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    line-height: 1.5;
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  .note {
    margin: 8px 0 0 44px;
    color: var(--color-danger);
  }

  @keyframes confirm-in {
    from {
      opacity: 0;
      translate: 0 -4px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .confirm {
      animation: none;
    }
  }
</style>

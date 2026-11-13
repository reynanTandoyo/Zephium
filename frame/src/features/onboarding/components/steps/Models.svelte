<script lang="ts">
  import { onMount } from "svelte";
  import { CommandLineIcon, CpuIcon, Key01Icon } from "@hugeicons/core-free-icons";
  import type { IconSvgElement } from "@hugeicons/svelte";
  import * as m from "$shared/i18n/messages";
  import { commands, type WorkLocalServerV1 } from "$shared/ipc/bindings";
  import { AiApps, appProviders, providerName } from "$domain/ai";
  import AiAppRow, { type AiAppView } from "$shared/ui/AiAppRow";
  import Button from "$shared/ui/Button";
  import Icon from "$shared/ui/Icon";
  import type { Mind } from "../../lib/minds";

  let {
    mind = $bindable(null),
    server = $bindable(null),
    discovering = $bindable(true),
    onsubmit,
  }: {
    mind?: Mind | null;
    /** A model server already running here, connected if chosen. */
    server?: WorkLocalServerV1 | null;
    discovering?: boolean;
    onsubmit: () => void;
  } = $props();

  const apps = new AiApps();
  /** The scan found an app that is signed in, so Work can open ready. */
  const ready = $derived(apps.apps?.find((app) => app.state === "ready") ?? null);
  const COULD_NOT_CHECK: AiAppView = {
    state: "unknown",
    version: null,
    install: { command: null, url: "" },
  };
  const rows = $derived(
    appProviders.map((provider) => {
      const app = apps.apps?.find((row) => row.provider === provider) ?? null;
      return { provider, app, view: app ?? (apps.unavailable ? COULD_NOT_CHECK : null) };
    }),
  );

  onMount(() => {
    void apps.scan(false);
    return () => apps.dispose();
  });

  onMount(() => {
    let live = true;
    if (!discovering) discovering = true;
    // Asks only this computer's own ports.
    void Promise.resolve()
      .then(() => commands.workLocalServers())
      .then((found) => {
        if (live) server = found.find((entry) => entry.models > 0) ?? null;
      })
      .catch(() => {})
      .finally(() => {
        if (live) discovering = false;
      });
    return () => (live = false);
  });

  const names = { ollama: "Ollama", lm_studio: "LM Studio" } as const;

  type Option = {
    id: Mind;
    icon: IconSvgElement;
    title: string;
    body: string;
    found: string | null;
  };

  const options = $derived.by<Option[]>(() => {
    return [
      {
        id: "app",
        icon: CommandLineIcon,
        title: m.onb_ai_app(),
        body: m.onb_ai_app_body(),
        found: ready ? m.onb_ai_app_found({ app: providerName(ready.provider) }) : null,
      },
      {
        id: "endpoint",
        icon: Key01Icon,
        title: m.onb_ai_endpoint(),
        body: m.onb_ai_endpoint_body(),
        found: null,
      },
      {
        id: "local",
        icon: CpuIcon,
        title: m.onb_ai_local(),
        body: m.onb_ai_local_body(),
        found: server ? m.onb_ai_found({ server: names[server.kind] }) : null,
      },
    ];
  });

  function keydown(event: KeyboardEvent) {
    if (event.key !== "Enter" || event.isComposing) return;
    event.preventDefault();
    onsubmit();
  }
</script>

<svelte:window onfocus={() => apps.returned()} />

<div class="options" role="radiogroup" aria-label={m.onb_ai_options()}>
  {#each options as option, index (option.id)}
    <label class="option" style:--i={index}>
      <input
        type="radio"
        name="onboarding-model"
        value={option.id}
        bind:group={mind}
        onkeydown={keydown}
      />
      <span class="top">
        <span class="tile"><Icon icon={option.icon} size={18} /></span>
        {#if option.found}<span class="found">{option.found}</span>{/if}
      </span>
      <span class="title">{option.title}</span>
      <span class="body">{option.body}</span>
    </label>
  {/each}
</div>

{#if mind === "app"}
  <div class="apps">
    {#each rows as row (row.provider)}<AiAppRow
        provider={row.provider}
        name={providerName(row.provider)}
        app={row.view}
        busy={!!row.app && apps.busy === row.app.app}
        failed={!!row.app && apps.failed === row.app.app}
        oninstall={() => row.app && void apps.install(row.app.app)}
        onsignin={() => row.app && void apps.signIn(row.app.app)}
      />{/each}
    <div class="check">
      <Button size="compact" pending={apps.scanning} onclick={() => void apps.scan(true)}
        >{m.ai_apps_check_again()}</Button
      >
    </div>
  </div>
{/if}

<style>
  .options {
    display: flex;
    gap: 16px;
  }

  .apps {
    box-sizing: border-box;

    /* As wide as the three cards above it and the gaps between them. */
    inline-size: calc(3 * 248px + 2 * 16px);
    margin-block-start: 16px;
    overflow: hidden;
    border-radius: var(--radius-card);
    background: var(--color-card);
    box-shadow: var(--shadow-raise);
    animation: lift 420ms var(--ease-emphasized) backwards;
  }

  .check {
    display: flex;
    justify-content: flex-end;
    padding: 10px 14px;
    border-block-start: 1px solid var(--color-border);
  }

  .option {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 6px;
    box-sizing: border-box;
    inline-size: 248px;
    min-block-size: 176px;
    padding: 18px 18px 20px;
    border-radius: var(--radius-card);
    background: var(--color-card);
    box-shadow: var(--shadow-raise);
    cursor: default;
    animation: lift 620ms var(--ease-emphasized) backwards;
    animation-delay: calc(300ms + var(--i) * 50ms);
    transition:
      background-color var(--motion-fast) var(--ease-out),
      box-shadow var(--motion-fast) var(--ease-out),
      scale var(--motion-slow) var(--ease-spring);
  }

  .option:hover {
    background: var(--color-fill-hover);
  }

  .option:active {
    scale: 0.985;
  }

  /* The radio is the control; the card is how it looks. */
  input {
    position: absolute;
    inset: 0;
    margin: 0;
    opacity: 0;
    cursor: default;
  }

  .option:has(input:checked) {
    background: var(--color-fill-active);
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--color-lit) 75%, transparent);
  }

  .option:has(input:focus-visible) {
    outline: 2px solid var(--color-ring);
    outline-offset: 2px;
  }

  .top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-block-end: 10px;
  }

  .tile {
    display: grid;
    inline-size: 36px;
    block-size: 36px;
    border-radius: var(--radius-control);
    background: var(--color-fill);
    color: var(--color-text);
    place-items: center;
  }

  .found {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--color-muted);
    font-size: 11.5px;
    font-weight: 500;
  }

  .found::before {
    content: "";
    inline-size: 6px;
    block-size: 6px;
    border-radius: 50%;
    background: var(--color-success);
  }

  .title {
    color: var(--color-text);
    font-size: 15px;
    font-weight: 500;
    letter-spacing: -0.01em;
  }

  .body {
    color: var(--color-muted);
    font-size: 13px;
    line-height: 1.45;
  }

  @keyframes lift {
    from {
      opacity: 0;
      transform: translateY(10px);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .option,
    .apps {
      animation: none;
    }
  }
</style>

<script lang="ts">
  import { onMount } from "svelte";
  import * as m from "$shared/i18n/messages";
  import type { WorkModelEntry, WorkModelRole } from "$shared/ipc/bindings";
  import { tabs } from "$domain/tabs";
  import {
    AiApps,
    ModelsSession,
    appProviders,
    entryLabel,
    entryOf,
    isApp,
    offeredProviders,
    pickerGroups,
    providerName,
    usable,
  } from "$domain/ai";
  import AiAppRow, { type AiAppView } from "$shared/ui/AiAppRow";
  import Button from "$shared/ui/Button";
  import Select from "$shared/ui/Select";
  import SettingsGroup from "$shared/ui/SettingsGroup";
  import SettingsRow from "$shared/ui/SettingsRow";
  import AiProviderRow from "../AiProviderRow.svelte";
  import AiVoiceRow from "../AiVoiceRow.svelte";
  import WorkDecisions from "../WorkDecisions.svelte";

  let profile = $derived(tabs.profile());
  let session = $state.raw<ModelsSession | null>(null);
  $effect(() => {
    const id = profile && profile.kind !== "incognito" ? profile.id : null;
    if (!id) return;
    const owner = new ModelsSession(id);
    session = owner;
    void owner.start();
    return () => {
      owner.dispose();
      if (session === owner) session = null;
    };
  });

  const apps = new AiApps();
  onMount(() => {
    void apps.scan(false);
    return () => apps.dispose();
  });
  /** What a row shows while the scan has no answer: checking, or that it could not check. */
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

  const models = $derived(session?.models ?? null);
  // The person's own server says which models it serves.
  $effect(() => {
    if (usable(models, "compatible")) void session?.listEndpoint();
  });
  const AUTO = "auto";
  let expanded = $state(false);
  const roles: {
    role: Exclude<WorkModelRole, "decision">;
    title: () => string;
    description: () => string;
  }[] = [
    { role: "lead", title: m.ai_role_lead, description: m.ai_role_lead_desc },
    { role: "page", title: m.ai_role_page, description: m.ai_role_page_byok_desc },
    { role: "light", title: m.ai_role_light, description: m.ai_role_light_desc },
  ];
  const anyUsable = $derived(
    !!models && offeredProviders.some((provider) => usable(models, provider)),
  );
  const customUsable = $derived(usable(models, "compatible"));

  function label(entry: WorkModelEntry) {
    const provider = entry.model.provider;
    const note = [
      entry.supports.vision ? m.ai_capability_vision() : null,
      entry.supports.native_search ? m.ai_capability_search() : null,
      entry.recommended && entry.roles.includes("light") ? m.ai_capability_fast() : null,
    ]
      .filter(Boolean)
      .join(" · ");
    const name = isApp(provider)
      ? entryLabel(entry)
      : provider === "anthropic" || provider === "deep_seek"
        ? entry.display_name
        : `${entry.display_name} · ${providerName(provider)}`;
    return note ? `${name} · ${note}` : name;
  }

  function options(role: Exclude<WorkModelRole, "decision">) {
    const effective = entryOf(models, models?.effective[role]);
    const automatic = {
      value: AUTO,
      label: effective ? m.ai_role_auto({ model: entryLabel(effective) }) : m.ai_role_auto_none(),
    };
    const offered = pickerGroups(models, role, [
      ...(session?.endpoint ?? []),
      ...(expanded ? (session?.more ?? []) : []),
    ]).ready.flatMap((group) => group.entries);
    return [automatic, ...offered.map((entry) => ({ value: entry.id, label: label(entry) }))];
  }
</script>

<svelte:window onfocus={() => apps.returned()} />

{#if !profile || profile.kind === "incognito"}<p class="ai-note">{m.work_regular_profile()}</p>
{:else if session?.unavailable && !models}<p class="ai-note" role="alert">{m.ai_unavailable()}</p>
{:else}
  {#if !anyUsable}<p class="ai-note">{m.ai_start_apps()}</p>{/if}

  <SettingsGroup title={m.ai_apps_group()} description={m.ai_apps_note()}>
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
  </SettingsGroup>

  <SettingsGroup title={m.ai_endpoint_group()} description={m.ai_endpoint_note()}>
    {#if session}<AiProviderRow {session} provider="compatible" />{/if}
  </SettingsGroup>

  <SettingsGroup
    title={m.ai_roles_group()}
    description={anyUsable ? m.ai_roles_note_apps() : m.ai_roles_none_apps()}
  >
    {#each roles as row (row.role)}
      <SettingsRow title={row.title()} description={row.description()}
        >{#key session?.fault?.action === `choose:${row.role}` ? session.fault : null}<Select
            label={row.title()}
            labelHidden
            options={options(row.role)}
            value={models?.chosen[row.role] ?? AUTO}
            disabled={!anyUsable || !!session?.busy}
            onchange={(value) => void session?.choose(row.role, value === AUTO ? null : value)}
          />{/key}</SettingsRow
      >
      {#if session?.fault?.action === `choose:${row.role}`}<p class="ai-note" role="alert">
          {m.work_model_failed()}
        </p>{/if}
    {/each}
    {#if customUsable}<div class="more">
        <Button
          size="compact"
          pending={session?.listingMore}
          onclick={() => {
            expanded = true;
            void session?.listMore(["compatible"]);
          }}>{m.ai_more_models()}</Button
        >
      </div>{/if}
    {#if session?.fault?.action === "more"}<p class="ai-note" role="alert">
        {m.ai_more_failed()}
      </p>{/if}
  </SettingsGroup>
  <AiVoiceRow />
  <WorkDecisions profile={profile.id} />
{/if}

<style>
  .ai-note {
    margin: 0 16px;
    font-size: var(--text-label);
    line-height: 1.5;
    color: var(--color-muted);
  }

  .more,
  .check {
    padding: 12px 14px;
  }

  .check {
    display: flex;
    justify-content: flex-end;
    border-top: 1px solid var(--color-border);
  }
</style>

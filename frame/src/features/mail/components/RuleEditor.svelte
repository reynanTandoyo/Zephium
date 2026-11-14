<!--
  Rules for one account: "if From / To / Subject contains X, then move it to
  a folder, mark it read or flag it". They run in this order on new Inbox mail
  after each sync, only on that account's own messages. A saved list is shown
  as saved only after Rust accepted it.
-->
<script lang="ts">
  import { onDestroy } from "svelte";
  import * as m from "$shared/i18n/messages";
  import type { MailRuleActionV1, MailRuleFieldV1 } from "$shared/ipc/bindings";
  import { MailRules, MAX_NEEDLE, MAX_RULES, ruleProblem, type MailStore } from "$domain/mail";
  import Button from "$shared/ui/Button";
  import Field from "$shared/ui/Field";
  import Select from "$shared/ui/Select";
  import { reasonText } from "../lib/text";

  let { store }: { store: MailStore } = $props();

  const rules = new MailRules();
  let account = $state("");

  onDestroy(() => rules.dispose());

  // Follow the account: the first one, until another is chosen or this one goes.
  $effect(() => {
    const known = store.accounts.map((candidate) => candidate.id);
    if (!known.length) return;
    if (!known.includes(account)) account = known[0]!;
  });
  $effect(() => {
    if (account) void rules.load(account);
  });

  const accounts = $derived(
    store.accounts.map((candidate) => ({ value: candidate.id, label: candidate.email })),
  );
  const fields = $derived<{ value: MailRuleFieldV1; label: string }[]>([
    { value: "from", label: m.mail_rule_field_from() },
    { value: "to", label: m.mail_rule_field_to() },
    { value: "subject", label: m.mail_rule_field_subject() },
  ]);
  const actions = $derived<{ value: MailRuleActionV1; label: string }[]>([
    { value: "move", label: m.mail_rule_action_move() },
    { value: "mark_read", label: m.mail_rule_action_mark_read() },
    { value: "flag", label: m.mail_rule_action_flag() },
  ]);
  const folders = $derived(
    store
      .foldersOf(account)
      .filter((folder) => folder.role !== "inbox")
      .map((folder) => ({ value: folder.path, label: folder.name })),
  );
  const busy = $derived(rules.state === "loading" || rules.state === "saving");

  function problemText(key: number) {
    const rule = rules.rules.find((candidate) => candidate.key === key);
    switch (rule ? ruleProblem(rule) : null) {
      case "empty":
        return m.mail_rule_problem_empty();
      case "too_long":
        return m.mail_rule_problem_too_long({ max: MAX_NEEDLE });
      case "no_folder":
        return m.mail_rule_problem_no_folder();
      default:
        return undefined;
    }
  }
</script>

<section class="rules" aria-labelledby="mail-rules-title">
  <h3 id="mail-rules-title">{m.mail_rules_title()}</h3>
  <p class="muted">{m.mail_rules_hint()}</p>

  {#if accounts.length > 1}
    <Select
      label={m.mail_rules_account()}
      options={accounts}
      value={account}
      onchange={(id) => (account = id)}
      disabled={busy}
    />
  {/if}

  {#if rules.state === "failed"}
    <div class="bad" role="alert">
      <p>{m.mail_rules_load_failed({ reason: reasonText(rules.failure ?? "internal") })}</p>
      <Button onclick={() => void rules.load(account)}>{m.mail_retry()}</Button>
    </div>
  {:else if rules.state === "loading"}
    <p class="muted" role="status">{m.mail_loading()}</p>
  {:else}
    {#if !rules.rules.length}<p class="muted">{m.mail_rules_empty()}</p>{/if}
    <ol>
      {#each rules.rules as rule, index (rule.key)}
        <li aria-label={m.mail_rule_number({ number: index + 1 })}>
          <span class="if">{m.mail_rule_if()}</span>
          <Select
            label={m.mail_rule_field()}
            labelHidden
            options={fields}
            value={rule.field}
            onchange={(value) => rules.update(rule.key, { field: value as MailRuleFieldV1 })}
            disabled={busy}
          />
          <span class="if">{m.mail_rule_contains()}</span>
          <Field
            label={m.mail_rule_text()}
            labelHidden
            maxlength={MAX_NEEDLE}
            spellcheck={false}
            autocomplete="off"
            value={rule.contains}
            error={rule.contains.length > 0 ? problemText(rule.key) : undefined}
            oninput={(event) => rules.update(rule.key, { contains: event.currentTarget.value })}
            disabled={busy}
          />
          <span class="if">{m.mail_rule_then()}</span>
          <Select
            label={m.mail_rule_action()}
            labelHidden
            options={actions}
            value={rule.action}
            onchange={(value) => rules.update(rule.key, { action: value as MailRuleActionV1 })}
            disabled={busy}
          />
          {#if rule.action === "move"}
            <Select
              label={m.mail_rule_folder()}
              labelHidden
              options={folders}
              value={rule.folder}
              onchange={(value) => rules.update(rule.key, { folder: value })}
              disabled={busy || !folders.length}
            />
          {/if}
          <div class="bar" role="group" aria-label={m.mail_rule_order({ number: index + 1 })}>
            <Button
              size="compact"
              disabled={index === 0 || busy}
              aria-label={m.mail_rule_move_up({ number: index + 1 })}
              onclick={() => rules.move(rule.key, -1)}>{m.mail_settings_move_up()}</Button
            >
            <Button
              size="compact"
              disabled={index === rules.rules.length - 1 || busy}
              aria-label={m.mail_rule_move_down({ number: index + 1 })}
              onclick={() => rules.move(rule.key, 1)}>{m.mail_settings_move_down()}</Button
            >
            <Button
              size="compact"
              variant="danger"
              disabled={busy}
              aria-label={m.mail_rule_remove({ number: index + 1 })}
              onclick={() => rules.remove(rule.key)}>{m.mail_rule_remove_short()}</Button
            >
          </div>
        </li>
      {/each}
    </ol>

    <div class="bar">
      <Button disabled={rules.full || busy} onclick={() => rules.add()}>{m.mail_rule_add()}</Button>
      <Button
        variant="primary"
        pending={rules.state === "saving"}
        disabled={busy || !rules.valid}
        onclick={() => void rules.save()}>{m.mail_rules_save()}</Button
      >
      {#if rules.full}<span class="muted">{m.mail_rules_full({ max: MAX_RULES })}</span>{/if}
      {#if rules.saved}<span class="muted" role="status">{m.mail_settings_saved()}</span>{/if}
    </div>
    {#if rules.failure}
      <p class="bad" role="alert">
        {m.mail_rules_save_failed({ reason: reasonText(rules.failure) })}
      </p>
    {/if}
  {/if}
</section>

<style>
  .rules {
    display: grid;
    gap: 12px;
    padding: 16px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
  }

  h3 {
    margin: 0;
    font-size: var(--text-page-title);
    font-weight: 600;
  }

  p {
    margin: 0;
  }

  ol {
    display: grid;
    gap: 12px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    align-items: start;
    gap: 8px;
    padding: 12px;
    border-radius: var(--radius-control);
    background: var(--color-fill);
  }

  .if {
    grid-column: 1 / -1;
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  li .bar {
    grid-column: 1 / -1;
  }

  .muted {
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  .bad {
    color: var(--color-danger);
    font-size: var(--text-body);
  }
</style>

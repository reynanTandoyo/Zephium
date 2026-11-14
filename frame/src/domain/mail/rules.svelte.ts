import { commands } from "$shared/ipc/bindings";
import type { MailRuleActionV1, MailRuleFieldV1, MailRuleV1 } from "$shared/ipc/bindings";
import { callMail, READ_TIMEOUT, WRITE_TIMEOUT, type MailFailure } from "./call";

/** Mirrors the bounds `zephium-mail` enforces; Rust refuses anything beyond them. */
export const MAX_RULES = 30;
export const MAX_NEEDLE = 120;

export type RuleDraft = {
  /** Local identity for keyed rendering only; never sent. */
  key: number;
  field: MailRuleFieldV1;
  contains: string;
  action: MailRuleActionV1;
  folder: string;
};

export type RuleProblem = "empty" | "too_long" | "no_folder";

/** Why a rule cannot be saved, or null when it can. */
export function ruleProblem(
  rule: Pick<RuleDraft, "contains" | "action" | "folder">,
): RuleProblem | null {
  const text = rule.contains.trim();
  // eslint-disable-next-line no-control-regex
  if (!text || /[\u0000-\u001f\u007f]/u.test(text)) return "empty";
  if (new TextEncoder().encode(rule.contains).length > MAX_NEEDLE) return "too_long";
  if (rule.action === "move" && !rule.folder) return "no_folder";
  return null;
}

export type RulesState = "idle" | "loading" | "ready" | "failed" | "saving";

/**
 * One account's rules being edited. Rust stores and applies them; this holds
 * the draft until a save is accepted. "Saved" follows the native answer.
 */
export class MailRules {
  account = $state("");
  rules = $state<readonly RuleDraft[]>([]);
  state = $state<RulesState>("idle");
  failure = $state<MailFailure | null>(null);
  saved = $state(false);
  private next = 0;
  private sequence = 0;
  private lifetime = new AbortController();

  get full() {
    return this.rules.length >= MAX_RULES;
  }

  /** Every rule is complete, so the list can be saved. */
  get valid() {
    return this.rules.every((rule) => ruleProblem(rule) === null);
  }

  private draft(rule: MailRuleV1): RuleDraft {
    return {
      key: this.next++,
      field: rule.field,
      contains: rule.contains,
      action: rule.action,
      folder: rule.folder ?? "",
    };
  }

  async load(account: string) {
    const sequence = ++this.sequence;
    this.account = account;
    this.state = "loading";
    this.failure = null;
    this.saved = false;
    this.rules = [];
    const outcome = await callMail(
      () => commands.mailRules(account),
      READ_TIMEOUT,
      this.lifetime.signal,
    );
    if (sequence !== this.sequence || outcome.ok === "aborted") return;
    if (!outcome.ok) {
      this.state = "failed";
      this.failure = outcome.error;
      return;
    }
    this.rules = outcome.value.map((rule) => this.draft(rule));
    this.state = "ready";
  }

  add() {
    if (this.full) return;
    this.rules = [
      ...this.rules,
      { key: this.next++, field: "from", contains: "", action: "mark_read", folder: "" },
    ];
    this.saved = false;
  }

  update(key: number, patch: Partial<Omit<RuleDraft, "key">>) {
    this.rules = this.rules.map((rule) => {
      if (rule.key !== key) return rule;
      const merged = { ...rule, ...patch };
      return merged.action === "move" ? merged : { ...merged, folder: "" };
    });
    this.saved = false;
  }

  remove(key: number) {
    this.rules = this.rules.filter((rule) => rule.key !== key);
    this.saved = false;
  }

  /** Rules run in list order, so moving one changes what wins. */
  move(key: number, step: -1 | 1) {
    const from = this.rules.findIndex((rule) => rule.key === key);
    const to = from + step;
    if (from < 0 || to < 0 || to >= this.rules.length) return;
    this.rules = this.rules.toSpliced(from, 1).toSpliced(to, 0, this.rules[from]!);
    this.saved = false;
  }

  async save(): Promise<boolean> {
    if (this.state !== "ready" || !this.valid) return false;
    const account = this.account;
    const sequence = this.sequence;
    this.state = "saving";
    this.failure = null;
    const wire: MailRuleV1[] = this.rules.map((rule) => ({
      field: rule.field,
      contains: rule.contains.trim(),
      action: rule.action,
      folder: rule.action === "move" ? rule.folder : null,
    }));
    const outcome = await callMail(
      () => commands.mailSetRules(account, wire),
      WRITE_TIMEOUT,
      this.lifetime.signal,
    );
    if (sequence !== this.sequence || outcome.ok === "aborted") return false;
    this.state = "ready";
    if (!outcome.ok) {
      this.failure = outcome.error;
      return false;
    }
    this.saved = true;
    return true;
  }

  dispose() {
    this.sequence++;
    this.lifetime.abort();
    this.lifetime = new AbortController();
  }
}

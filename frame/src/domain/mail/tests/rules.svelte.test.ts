import { afterEach, expect, test, vi } from "vitest";

const host = vi.hoisted(() => ({ rules: vi.fn(), setRules: vi.fn() }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ mailRules: host.rules, mailSetRules: host.setRules });
});

const { MAX_NEEDLE, MAX_RULES, MailRules, ruleProblem } = await import("../rules.svelte");

const ok = <T>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });

afterEach(() => {
  host.rules.mockReset();
  host.setRules.mockReset();
});

test("a rule needs text within bounds, and a folder to move to", () => {
  const base = { contains: "boss", action: "flag" as const, folder: "" };
  expect(ruleProblem(base)).toBeNull();
  expect(ruleProblem({ ...base, contains: "   " })).toBe("empty");
  expect(ruleProblem({ ...base, contains: "a\nb" })).toBe("empty");
  expect(ruleProblem({ ...base, contains: "x".repeat(MAX_NEEDLE + 1) })).toBe("too_long");
  expect(ruleProblem({ ...base, contains: "ü".repeat(MAX_NEEDLE / 2 + 1) })).toBe("too_long");
  expect(ruleProblem({ ...base, action: "move" })).toBe("no_folder");
  expect(ruleProblem({ ...base, action: "move", folder: "Bills" })).toBeNull();
});

test("loading shows the stored rules in order; a failed load says why", async () => {
  host.rules.mockReturnValue(
    ok([
      { field: "from", contains: "boss", action: "flag", folder: null },
      { field: "subject", contains: "invoice", action: "move", folder: "Bills" },
    ]),
  );
  const rules = new MailRules();
  await rules.load("acc-1");
  expect(host.rules).toHaveBeenCalledWith("acc-1");
  expect(rules.state).toBe("ready");
  expect(rules.rules.map((r) => [r.contains, r.folder])).toEqual([
    ["boss", ""],
    ["invoice", "Bills"],
  ]);
  host.rules.mockReturnValue(err("busy"));
  await rules.load("acc-1");
  expect(rules.state).toBe("failed");
  expect(rules.failure).toBe("busy");
  expect(rules.rules).toEqual([]);
  rules.dispose();
});

test("rules are edited, reordered and bounded locally, and saved exactly as shown", async () => {
  host.rules.mockReturnValue(ok([]));
  host.setRules.mockReturnValue(ok(null));
  const rules = new MailRules();
  await rules.load("acc-1");
  rules.add();
  rules.add();
  const [first, second] = rules.rules;
  expect(rules.valid).toBe(false);
  expect(await rules.save()).toBe(false);
  expect(host.setRules).not.toHaveBeenCalled();
  rules.update(first!.key, { contains: "  boss ", action: "flag" });
  rules.update(second!.key, {
    field: "subject",
    contains: "invoice",
    action: "move",
    folder: "Bills",
  });
  expect(rules.valid).toBe(true);
  // Leaving "move" forgets the folder, so a stale one is never sent.
  rules.update(second!.key, { action: "flag" });
  expect(rules.rules[1]!.folder).toBe("");
  rules.update(second!.key, { action: "move", folder: "Bills" });
  rules.move(second!.key, -1);
  rules.move(second!.key, -1);
  expect(rules.rules.map((r) => r.contains)).toEqual(["invoice", "  boss "]);
  expect(await rules.save()).toBe(true);
  expect(host.setRules).toHaveBeenCalledExactlyOnceWith("acc-1", [
    { field: "subject", contains: "invoice", action: "move", folder: "Bills" },
    { field: "from", contains: "boss", action: "flag", folder: null },
  ]);
  expect(rules.saved).toBe(true);
  rules.remove(first!.key);
  expect(rules.saved).toBe(false);
  expect(rules.rules).toHaveLength(1);
  rules.dispose();
});

test("saving is only claimed after the answer, and a refusal keeps the draft", async () => {
  host.rules.mockReturnValue(ok([{ field: "from", contains: "a", action: "flag", folder: null }]));
  let release!: (value: unknown) => void;
  host.setRules.mockReturnValue(new Promise((resolve) => (release = resolve)));
  const rules = new MailRules();
  await rules.load("acc-1");
  const saving = rules.save();
  expect(rules.state).toBe("saving");
  expect(rules.saved).toBe(false);
  release({ status: "error", error: "too_large" });
  expect(await saving).toBe(false);
  expect(rules.state).toBe("ready");
  expect(rules.failure).toBe("too_large");
  expect(rules.saved).toBe(false);
  expect(rules.rules).toHaveLength(1);
  rules.dispose();
});

test("the list stops at its bound", async () => {
  host.rules.mockReturnValue(ok([]));
  const rules = new MailRules();
  await rules.load("acc-1");
  for (let n = 0; n < MAX_RULES + 5; n++) rules.add();
  expect(rules.rules).toHaveLength(MAX_RULES);
  expect(rules.full).toBe(true);
  rules.dispose();
});

test("an answer for an account that is no longer shown is ignored", async () => {
  let releaseFirst!: (value: unknown) => void;
  host.rules
    .mockReturnValueOnce(new Promise((resolve) => (releaseFirst = resolve)))
    .mockReturnValueOnce(ok([{ field: "to", contains: "two", action: "flag", folder: null }]));
  const rules = new MailRules();
  const first = rules.load("acc-1");
  await rules.load("acc-2");
  releaseFirst({
    status: "ok",
    data: [{ field: "from", contains: "one", action: "flag", folder: null }],
  });
  await first;
  expect(rules.account).toBe("acc-2");
  expect(rules.rules.map((r) => r.contains)).toEqual(["two"]);
  rules.dispose();
});

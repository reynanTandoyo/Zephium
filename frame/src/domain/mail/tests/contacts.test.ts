import { afterEach, beforeEach, expect, test, vi } from "vitest";

const host = vi.hoisted(() => ({ contacts: vi.fn() }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ mailContacts: host.contacts });
});

const { completeEntry, typedEntry } = await import("../address");
const { ContactSuggester, SUGGESTION_LIMIT } = await import("../contacts.svelte");

const ok = <T>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });
const contact = (email: string, name: string | null = null) => ({ name, email });

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  vi.useRealTimers();
  host.contacts.mockReset();
});

test("the entry being typed is what follows the last separator, and none once it is complete", () => {
  expect(typedEntry("")).toBe("");
  expect(typedEntry("an")).toBe("an");
  expect(typedEntry("ann@example.org, bo")).toBe("bo");
  expect(typedEntry("ann@example.org; bo ")).toBe("bo");
  expect(typedEntry("a@x.org,\n  bo")).toBe("bo");
  expect(typedEntry("ann@example.org, ")).toBe("");
  expect(typedEntry("Ann <ann@example.org>")).toBe("");
});

test("choosing a suggestion replaces only the entry being typed", () => {
  const ann = contact("ann@example.org", "Ann Example");
  expect(completeEntry("an", ann)).toBe("Ann Example <ann@example.org>, ");
  expect(completeEntry("bob@example.org, an", ann)).toBe(
    "bob@example.org, Ann Example <ann@example.org>, ",
  );
  expect(completeEntry("bob@example.org,an", ann)).toBe(
    "bob@example.org, Ann Example <ann@example.org>, ",
  );
  expect(completeEntry("a", contact("plain@example.org"))).toBe("plain@example.org, ");
  // A name cannot smuggle brackets or quotes into the address list.
  expect(completeEntry("x", contact("a@example.org", 'Eve <"x">'))).toBe("Eve x <a@example.org>, ");
});

test("typing asks for the entry once, after a pause, and publishes the answer", async () => {
  host.contacts.mockReturnValue(ok([contact("ann@example.org", "Ann")]));
  const suggester = new ContactSuggester(100);
  suggester.request("a");
  suggester.request("an");
  suggester.request("ann");
  expect(host.contacts).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(100);
  expect(host.contacts).toHaveBeenCalledExactlyOnceWith("ann", SUGGESTION_LIMIT);
  expect(suggester.results).toEqual([contact("ann@example.org", "Ann")]);
  suggester.dispose();
});

test("an empty or finished entry asks nothing and clears what was shown", async () => {
  host.contacts.mockReturnValue(ok([contact("ann@example.org")]));
  const suggester = new ContactSuggester(10);
  suggester.request("ann");
  await vi.advanceTimersByTimeAsync(10);
  expect(suggester.results).toHaveLength(1);
  suggester.request("ann@example.org, ");
  expect(suggester.results).toEqual([]);
  await vi.advanceTimersByTimeAsync(50);
  expect(host.contacts).toHaveBeenCalledTimes(1);
  suggester.dispose();
});

test("an older answer never replaces a newer one, and a failure shows nothing", async () => {
  let releaseFirst!: (value: unknown) => void;
  host.contacts
    .mockReturnValueOnce(new Promise((resolve) => (releaseFirst = resolve)))
    .mockReturnValueOnce(ok([contact("bob@example.org")]))
    .mockReturnValueOnce(err("internal"));
  const suggester = new ContactSuggester(10);
  suggester.request("a");
  await vi.advanceTimersByTimeAsync(10);
  suggester.request("b");
  await vi.advanceTimersByTimeAsync(10);
  expect(suggester.results).toEqual([contact("bob@example.org")]);
  releaseFirst({ status: "ok", data: [contact("ann@example.org")] });
  await vi.advanceTimersByTimeAsync(0);
  expect(suggester.results).toEqual([contact("bob@example.org")]);
  suggester.request("c");
  await vi.advanceTimersByTimeAsync(10);
  expect(suggester.results).toEqual([]);
  suggester.dispose();
});

test("disposing drops a pending request and an answer still on its way", async () => {
  let release!: (value: unknown) => void;
  host.contacts.mockReturnValue(new Promise((resolve) => (release = resolve)));
  const suggester = new ContactSuggester(10);
  suggester.request("a");
  await vi.advanceTimersByTimeAsync(10);
  suggester.dispose();
  release({ status: "ok", data: [contact("late@example.org")] });
  await vi.advanceTimersByTimeAsync(0);
  expect(suggester.results).toEqual([]);
  suggester.request("b");
  suggester.dispose();
  await vi.advanceTimersByTimeAsync(50);
  expect(host.contacts).toHaveBeenCalledTimes(1);
});

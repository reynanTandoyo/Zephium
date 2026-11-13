import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page, userEvent } from "vitest/browser";
import "$styles/global.css";
import { mailtoInbox, MailStore } from "$domain/mail";
import AddressField from "../components/AddressField.svelte";
import MailPage from "../components/MailPage.svelte";
import RuleEditor from "../components/RuleEditor.svelte";
import { account, failed, folders, message, ok, pageOf } from "./fixtures";

const native = vi.hoisted(() => ({
  contacts: vi.fn(),
  rules: vi.fn(),
  setRules: vi.fn(),
  accounts: vi.fn(),
  folders: vi.fn(),
  messages: vi.fn(),
  sync: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    mailContacts: native.contacts,
    mailRules: native.rules,
    mailSetRules: native.setRules,
    mailAccounts: native.accounts,
    mailFolders: native.folders,
    mailMessages: native.messages,
    mailSync: native.sync,
  });
});

afterEach(() => {
  for (const mock of Object.values(native)) mock.mockReset();
  mailtoInbox.take();
});

const ann = { name: "Ann Example", email: "ann@example.org" };
const anders = { name: null, email: "anders@example.org" };

// Contact suggestions ----------------------------------------------------------

test("typing lists suggestions from the cache; arrow keys and Enter choose one", async () => {
  native.contacts.mockResolvedValue(ok([ann, anders]));
  render(AddressField, { label: "To" });
  const input = page.getByRole("combobox", { name: "To" });
  await expect.element(input).toHaveAttribute("aria-expanded", "false");
  await input.fill("bob@example.org, an");
  await expect.element(page.getByRole("listbox")).toBeVisible();
  expect(native.contacts).toHaveBeenLastCalledWith("an", 8);
  await expect.element(page.getByRole("option", { name: /Ann Example/u })).toBeVisible();
  await expect.element(page.getByRole("option", { name: "anders@example.org" })).toBeVisible();
  await expect.element(input).toHaveAttribute("aria-expanded", "true");
  await userEvent.keyboard("{ArrowDown}");
  await expect
    .element(page.getByRole("option", { name: /Ann Example/u }))
    .toHaveAttribute("aria-selected", "true");
  await userEvent.keyboard("{ArrowDown}{ArrowUp}");
  await userEvent.keyboard("{Enter}");
  await expect.element(input).toHaveValue("bob@example.org, Ann Example <ann@example.org>, ");
  await expect.element(page.getByRole("listbox")).not.toBeInTheDocument();
});

test("a suggestion can be picked with the pointer, and Tab takes the highlighted one", async () => {
  native.contacts.mockResolvedValue(ok([ann, anders]));
  render(AddressField, { label: "To" });
  const input = page.getByRole("combobox", { name: "To" });
  await input.fill("an");
  await page.getByRole("option", { name: "anders@example.org" }).click();
  await expect.element(input).toHaveValue("anders@example.org, ");
  await input.fill("an");
  await expect.element(page.getByRole("listbox")).toBeVisible();
  await userEvent.keyboard("{ArrowDown}{Tab}");
  await expect.element(input).toHaveValue("Ann Example <ann@example.org>, ");
});

test("Escape closes only the list, Enter without a highlight chooses nothing", async () => {
  native.contacts.mockResolvedValue(ok([ann]));
  const outer = vi.fn();
  document.addEventListener("keydown", outer);
  render(AddressField, { label: "To" });
  const input = page.getByRole("combobox", { name: "To" });
  await input.fill("an");
  await expect.element(page.getByRole("listbox")).toBeVisible();
  await userEvent.keyboard("{Enter}");
  await expect.element(input).toHaveValue("an");
  await userEvent.keyboard("{Escape}");
  await expect.element(page.getByRole("listbox")).not.toBeInTheDocument();
  expect(outer.mock.calls.some(([event]) => (event as KeyboardEvent).key === "Escape")).toBe(false);
  document.removeEventListener("keydown", outer);
});

test("addresses already in the field are not offered again, and a failed lookup shows nothing", async () => {
  native.contacts.mockResolvedValue(ok([ann, anders]));
  render(AddressField, { label: "To", value: "ann@example.org, an" });
  const input = page.getByRole("combobox", { name: "To" });
  await input.fill("ann@example.org, an");
  await expect.element(page.getByRole("option", { name: "anders@example.org" })).toBeVisible();
  await expect.element(page.getByRole("option", { name: /Ann Example/u })).not.toBeInTheDocument();
  native.contacts.mockResolvedValue(failed("internal"));
  await input.fill("ann@example.org, zz");
  await expect.element(page.getByRole("listbox")).not.toBeInTheDocument();
});

test("nothing is asked for while the field is empty or an address is complete", async () => {
  native.contacts.mockResolvedValue(ok([ann]));
  render(AddressField, { label: "To" });
  const input = page.getByRole("combobox", { name: "To" });
  await input.fill("ann@example.org, ");
  await new Promise((done) => setTimeout(done, 300));
  expect(native.contacts).not.toHaveBeenCalled();
  await expect.element(page.getByRole("listbox")).not.toBeInTheDocument();
});

// Offline -------------------------------------------------------------------------

function start(from_cache: boolean) {
  native.accounts.mockResolvedValue(ok([account]));
  native.folders.mockResolvedValue(ok(folders));
  native.messages.mockResolvedValue(
    ok({ messages: [message(1)], next_before: null, total: 1, from_cache }),
  );
}

test("Offline shows while the list is the saved copy and goes when the server answers", async () => {
  start(true);
  native.sync.mockResolvedValue(ok(folders));
  render(MailPage);
  await expect.element(page.getByText("Ann Example")).toBeVisible();
  await expect.element(page.getByRole("status").filter({ hasText: /^Offline$/u })).toBeVisible();
  await expect.element(page.getByText("Offline: showing saved messages")).toBeVisible();
  native.messages.mockResolvedValue(pageOf([message(1)]));
  await page.getByRole("button", { name: /Sync/u }).click();
  await expect
    .element(page.getByRole("status").filter({ hasText: /^Offline$/u }))
    .not.toBeInTheDocument();
});

test("a sync that cannot reach the server shows Offline and keeps the folders", async () => {
  start(false);
  native.sync.mockResolvedValue(failed("offline"));
  render(MailPage);
  await expect.element(page.getByText("Ann Example")).toBeVisible();
  await expect
    .element(page.getByRole("status").filter({ hasText: /^Offline$/u }))
    .not.toBeInTheDocument();
  await page.getByRole("button", { name: /Sync/u }).click();
  await expect.element(page.getByRole("status").filter({ hasText: /^Offline$/u })).toBeVisible();
  await expect.element(page.getByText("Ann Example")).toBeVisible();
});

// mailto ---------------------------------------------------------------------------

test("a clicked mailto link opens a prefilled message", async () => {
  start(false);
  mailtoInbox.receive({
    to: ["ann@example.org"],
    cc: ["cy@example.org"],
    bcc: [],
    subject: "Hello from a page",
    body: "Line one",
  });
  render(MailPage);
  await expect.element(page.getByLabelText("To", { exact: true })).toHaveValue("ann@example.org");
  await expect.element(page.getByLabelText("Cc", { exact: true })).toHaveValue("cy@example.org");
  await expect.element(page.getByLabelText("Subject")).toHaveValue("Hello from a page");
  await expect
    .element(page.getByRole("textbox", { name: "Message", exact: true }))
    .toHaveValue("Line one\n");
  expect(mailtoInbox.pending).toBeNull();
});

// Rules ----------------------------------------------------------------------------

function editor() {
  const store = new MailStore();
  store.accounts = [account];
  store.folders = { [account.id]: folders };
  render(RuleEditor, { store });
  return store;
}

test("stored rules load in order and a new rule is saved exactly as typed", async () => {
  native.rules.mockResolvedValue(
    ok([{ field: "from", contains: "boss", action: "flag", folder: null }]),
  );
  native.setRules.mockResolvedValue(ok(null));
  editor();
  await expect.element(page.getByLabelText("Text to look for").first()).toHaveValue("boss");
  await page.getByRole("button", { name: "Add rule" }).click();
  await expect.element(page.getByText("Enter the text to look for.")).not.toBeInTheDocument();
  // An incomplete rule cannot be saved.
  await expect.element(page.getByRole("button", { name: "Save rules" })).toBeDisabled();
  await page.getByLabelText("Text to look for").nth(1).fill("Newsletter");
  await page.getByRole("button", { name: "Save rules" }).click();
  expect(native.setRules).toHaveBeenCalledExactlyOnceWith("acc-1", [
    { field: "from", contains: "boss", action: "flag", folder: null },
    { field: "from", contains: "Newsletter", action: "mark_read", folder: null },
  ]);
  await expect.element(page.getByText("Saved.")).toBeVisible();
});

test("a move rule needs a folder, and choosing one enables saving", async () => {
  native.rules.mockResolvedValue(ok([]));
  native.setRules.mockResolvedValue(ok(null));
  editor();
  await page.getByRole("button", { name: "Add rule" }).click();
  await page.getByLabelText("Text to look for").fill("invoice");
  await page.getByRole("button", { name: "Action" }).click();
  await page.getByRole("option", { name: "Move to folder" }).click();
  await expect.element(page.getByText("Choose a folder.")).not.toBeInTheDocument();
  await expect.element(page.getByRole("button", { name: "Save rules" })).toBeDisabled();
  await page.getByRole("button", { name: "Folder" }).click();
  await page.getByRole("option", { name: "Archive" }).click();
  await page.getByRole("button", { name: "Save rules" }).click();
  expect(native.setRules).toHaveBeenCalledWith("acc-1", [
    { field: "from", contains: "invoice", action: "move", folder: "Archive" },
  ]);
});

test("rules are reordered and removed, and a refused save is reported", async () => {
  native.rules.mockResolvedValue(
    ok([
      { field: "from", contains: "one", action: "flag", folder: null },
      { field: "subject", contains: "two", action: "mark_read", folder: null },
    ]),
  );
  native.setRules.mockResolvedValue(failed("too_large"));
  editor();
  await expect.element(page.getByLabelText("Text to look for").first()).toHaveValue("one");
  await page.getByRole("button", { name: "Move rule 2 up" }).click();
  await expect.element(page.getByLabelText("Text to look for").first()).toHaveValue("two");
  await page.getByRole("button", { name: "Remove rule 2" }).click();
  await expect.element(page.getByLabelText("Text to look for")).toHaveValue("two");
  await page.getByRole("button", { name: "Save rules" }).click();
  await expect.element(page.getByRole("alert")).toHaveTextContent(/Could not save the rules/u);
  expect(native.setRules).toHaveBeenCalledWith("acc-1", [
    { field: "subject", contains: "two", action: "mark_read", folder: null },
  ]);
  await expect.element(page.getByText("Saved.")).not.toBeInTheDocument();
});

test("a rule list that cannot be read says why and offers a retry", async () => {
  native.rules.mockResolvedValueOnce(failed("busy"));
  native.rules.mockResolvedValue(ok([]));
  editor();
  await expect.element(page.getByRole("alert")).toHaveTextContent(/Could not load the rules/u);
  await page.getByRole("button", { name: "Try again" }).click();
  await expect.element(page.getByText("No rules yet.")).toBeVisible();
});

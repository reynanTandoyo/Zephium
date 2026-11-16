import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page, userEvent } from "vitest/browser";
import "$styles/global.css";
import MailPage from "../components/MailPage.svelte";
import { account, body, failed, folders, message, ok, pageOf } from "./fixtures";

const native = vi.hoisted(() => ({
  accounts: vi.fn(),
  folders: vi.fn(),
  messages: vi.fn(),
  message: vi.fn(),
  setFlags: vi.fn(),
  move: vi.fn(),
  del: vi.fn(),
  autoconfig: vi.fn(),
  test: vi.fn(),
  add: vi.fn(),
  watch: vi.fn(),
  search: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    mailAccounts: native.accounts,
    mailFolders: native.folders,
    mailMessages: native.messages,
    mailMessage: native.message,
    mailSetFlags: native.setFlags,
    mailMove: native.move,
    mailDelete: native.del,
    mailAutoconfig: native.autoconfig,
    mailTestAccount: native.test,
    mailAddAccount: native.add,
    mailWatch: native.watch,
    mailSearch: native.search,
  });
});

afterEach(() => {
  for (const mock of Object.values(native)) mock.mockReset();
});

function withAccount(...messages: ReturnType<typeof message>[]) {
  native.accounts.mockResolvedValue(ok([account]));
  native.folders.mockResolvedValue(ok(folders));
  native.messages.mockResolvedValue(pageOf(messages));
}

const unseen = { seen: false, flagged: false, answered: false, draft: false, deleted: false };

test("with no account the setup comes first and the password is not kept after submitting", async () => {
  native.accounts.mockResolvedValue(ok([]));
  native.autoconfig.mockResolvedValue(
    ok({
      found: true,
      imap: { host: "imap.example.com", port: 993, security: "tls" },
      smtp: { host: "smtp.example.com", port: 465, security: "tls" },
      username: "me@example.com",
      provider: "Example",
      source: "ispdb",
    }),
  );
  native.test.mockResolvedValue(ok({ imap: null, smtp: null }));
  native.add.mockResolvedValue(failed("credentials"));
  render(MailPage);
  await expect.element(page.getByRole("heading", { name: "Add your email account" })).toBeVisible();
  await page.getByLabelText("Email address").fill("me@example.com");
  const password = page.getByLabelText("Password", { exact: true });
  await password.fill("hunter2");
  await page.getByRole("button", { name: "Find servers" }).click();
  expect(native.autoconfig).toHaveBeenCalledWith("me@example.com");
  await expect.element(page.getByText("Servers found: Example")).toBeVisible();
  await expect.element(page.getByRole("button", { name: "Add account" })).toBeDisabled();
  await page.getByRole("button", { name: "Test connection" }).click();
  await expect.element(page.getByText("Both servers accepted the connection.")).toBeVisible();
  await page.getByRole("button", { name: "Add account" }).click();
  expect(native.add.mock.calls[0]![0]).toMatchObject({
    email: "me@example.com",
    imap: { host: "imap.example.com" },
  });
  await expect.element(page.getByText(/saved password could not be used/u)).toBeVisible();
  await expect.element(password).toHaveValue("");
});

test("a lookup that finds nothing offers the servers by hand", async () => {
  native.accounts.mockResolvedValue(ok([]));
  native.autoconfig.mockResolvedValue(
    ok({ found: false, imap: null, smtp: null, username: null, provider: null, source: null }),
  );
  render(MailPage);
  await page.getByLabelText("Email address").fill("me@corp.example");
  await page.getByRole("button", { name: "Find servers" }).click();
  await expect.element(page.getByText(/No settings are known/u)).toBeVisible();
  await expect.element(page.getByText("Incoming server (IMAP)")).toBeVisible();
  await expect.element(page.getByText("Outgoing server (SMTP)")).toBeVisible();
});

test("the list shows sender, subject, snippet and marks, and a click opens the message", async () => {
  const first = message(1, { flags: unseen, has_attachments: true });
  withAccount(
    first,
    message(2, { subject: "Second", from: [{ name: null, email: "bob@example.org" }] }),
  );
  native.message.mockResolvedValue(ok(body(first, { text: "Hello there" })));
  native.setFlags.mockResolvedValue(ok(null));
  render(MailPage);
  await expect.element(page.getByText("Ann Example")).toBeVisible();
  await expect.element(page.getByText("bob@example.org")).toBeVisible();
  await expect.element(page.getByText("Snippet 1")).toBeVisible();
  await expect.element(page.getByRole("img", { name: "Has attachments" })).toBeVisible();
  await expect.element(page.getByRole("img", { name: "Unread" })).toBeVisible();
  await page.getByRole("button", { name: /Subject 1/u }).click();
  await expect.element(page.getByText("Hello there")).toBeVisible();
  expect(native.message).toHaveBeenCalledWith("acc-1", "INBOX", 1);
  // Opening marks it read, and the unread mark goes only after the server agreed.
  await vi.waitFor(() =>
    expect(native.setFlags).toHaveBeenCalledWith("acc-1", "INBOX", [1], "seen", true),
  );
  await expect.element(page.getByRole("img", { name: "Unread" })).not.toBeInTheDocument();
});

test("conversations are grouped into one row", async () => {
  withAccount(
    message(1, { subject: "Plan" }),
    message(2, { subject: "Re: Plan", in_reply_to: "<m1@x>", references: ["<m1@x>"] }),
  );
  render(MailPage);
  await expect.element(page.getByText("Plan", { exact: true })).toBeVisible();
  await expect.element(page.getByTitle("2 messages in this conversation")).toBeVisible();
});

test("a failed list says why and retries", async () => {
  native.accounts.mockResolvedValue(ok([account]));
  native.folders.mockResolvedValue(ok(folders));
  native.messages.mockResolvedValueOnce(failed("offline"));
  native.messages.mockResolvedValue(pageOf([message(1)]));
  render(MailPage);
  await expect.element(page.getByText(/The server cannot be reached/u)).toBeVisible();
  await page.getByRole("button", { name: "Try again" }).click();
  await expect.element(page.getByText("Ann Example")).toBeVisible();
});

test("a refused delete keeps the message and explains", async () => {
  const first = message(1);
  withAccount(first);
  native.message.mockResolvedValue(ok(body(first)));
  native.del.mockResolvedValue(failed("server_rejected"));
  render(MailPage);
  await page.getByRole("button", { name: /Subject 1/u }).click();
  await page.getByRole("button", { name: "Delete (#)" }).click();
  expect(native.del).toHaveBeenCalledWith("acc-1", "INBOX", [1], false);
  await expect.element(page.getByText(/The server refused the request/u)).toBeVisible();
  // The message stays open, and nothing was removed on the server's refusal.
  await expect.element(page.getByRole("heading", { name: "Subject 1" })).toBeVisible();
});

test("deleting where there is no Trash asks first, and only a yes removes anything", async () => {
  const first = message(1);
  withAccount(first);
  native.message.mockResolvedValue(ok(body(first)));
  native.del.mockResolvedValueOnce(failed("no_trash"));
  native.del.mockResolvedValueOnce(failed("no_trash"));
  native.del.mockResolvedValueOnce(ok(null));
  render(MailPage);
  await page.getByRole("button", { name: /Subject 1/u }).click();
  await page.getByRole("button", { name: "Delete (#)" }).click();
  expect(native.del).toHaveBeenCalledTimes(1);
  expect(native.del).toHaveBeenLastCalledWith("acc-1", "INBOX", [1], false);
  await expect.element(page.getByRole("alertdialog")).toBeVisible();
  await expect.element(page.getByText(/no Trash folder, so deleting/u)).toBeVisible();
  // Cancelling sends nothing more and keeps the message.
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect.element(page.getByRole("alertdialog")).not.toBeInTheDocument();
  expect(native.del).toHaveBeenCalledTimes(1);
  await expect.element(page.getByRole("heading", { name: "Subject 1" })).toBeVisible();
  // Asking again and confirming sends the permanent delete.
  await page.getByRole("button", { name: "Delete (#)" }).click();
  await page.getByRole("button", { name: "Delete permanently" }).click();
  await vi.waitFor(() => expect(native.del).toHaveBeenCalledTimes(3));
  expect(native.del).toHaveBeenLastCalledWith("acc-1", "INBOX", [1], true);
});

test("search runs against the current folder", async () => {
  withAccount(message(1));
  native.search.mockResolvedValue(ok([message(5, { subject: "Invoice" })]));
  render(MailPage);
  await expect.element(page.getByText("Ann Example")).toBeVisible();
  await page.getByRole("button", { name: /^Inbox/u }).click();
  await page.getByLabelText("Search this folder").fill("invoice");
  await vi.waitFor(() => expect(native.search).toHaveBeenCalledWith("acc-1", "INBOX", "invoice"));
  await expect.element(page.getByText("Invoice", { exact: true })).toBeVisible();
});

test("shortcut keys act on the open message and tooltips name them", async () => {
  const first = message(1);
  withAccount(first);
  native.message.mockResolvedValue(ok(body(first)));
  native.setFlags.mockResolvedValue(ok(null));
  render(MailPage);
  await page.getByRole("button", { name: /Subject 1/u }).click();
  await expect.element(page.getByRole("button", { name: "Reply (R)" })).toBeVisible();
  await userEvent.keyboard("s");
  await vi.waitFor(() =>
    expect(native.setFlags).toHaveBeenCalledWith("acc-1", "INBOX", [1], "flagged", true),
  );
});

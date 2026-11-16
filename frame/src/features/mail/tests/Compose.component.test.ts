import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import "$styles/global.css";
import { mailtoInbox } from "$domain/mail";
import MailPage from "../components/MailPage.svelte";
import { account, body, failed, folders, message, ok, pageOf } from "./fixtures";

const native = vi.hoisted(() => ({
  accounts: vi.fn(),
  folders: vi.fn(),
  messages: vi.fn(),
  message: vi.fn(),
  setFlags: vi.fn(),
  send: vi.fn(),
  saveDraft: vi.fn(),
  contacts: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    mailAccounts: native.accounts,
    mailFolders: native.folders,
    mailMessages: native.messages,
    mailMessage: native.message,
    mailSetFlags: native.setFlags,
    mailSend: native.send,
    mailSaveDraft: native.saveDraft,
    mailContacts: native.contacts,
  });
});

afterEach(() => {
  for (const mock of Object.values(native)) mock.mockReset();
  mailtoInbox.take();
});

function start(signature = "") {
  native.accounts.mockResolvedValue(ok([{ ...account, signature }]));
  native.folders.mockResolvedValue(ok(folders));
  native.messages.mockResolvedValue(pageOf([message(1)]));
  native.contacts.mockResolvedValue(ok([]));
}

test("a new message is sent with the typed recipients; success shows only after the answer", async () => {
  start("Best, Me");
  let release!: (value: unknown) => void;
  native.send.mockReturnValue(new Promise((resolve) => (release = resolve)));
  render(MailPage);
  await page.getByRole("button", { name: /New message/u }).click();
  await expect
    .element(page.getByRole("textbox", { name: "Message", exact: true }))
    .toHaveValue("\n-- \nBest, Me\n");
  await page.getByLabelText("To", { exact: true }).fill("Ann <ann@example.org>, bob@example.org");
  await page.getByLabelText("Subject").fill("Hello");
  await page.getByRole("button", { name: "Send" }).click();
  expect(native.send.mock.calls[0]![0]).toBe("acc-1");
  expect(native.send.mock.calls[0]![1]).toMatchObject({
    to: [
      { name: "Ann", email: "ann@example.org" },
      { name: null, email: "bob@example.org" },
    ],
    subject: "Hello",
    html: null,
  });
  await expect.element(page.getByRole("button", { name: "Sending…" })).toBeVisible();
  expect(document.body.textContent).not.toContain("Message sent.");
  release(ok({ sent: true, saved_copy: true }));
  await expect.element(page.getByText("Message sent.")).toBeVisible();
});

test("a failed send says why and keeps the text for another try", async () => {
  start();
  native.send.mockResolvedValue(failed("auth_failed"));
  render(MailPage);
  await page.getByRole("button", { name: /New message/u }).click();
  await page.getByLabelText("To", { exact: true }).fill("ann@example.org");
  await page.getByRole("textbox", { name: "Message", exact: true }).fill("keep me");
  await page.getByRole("button", { name: "Send" }).click();
  await expect
    .element(page.getByText(/The message was not sent: The server did not accept/u))
    .toBeVisible();
  await expect
    .element(page.getByRole("textbox", { name: "Message", exact: true }))
    .toHaveValue("keep me");
});

test("a missing recipient or a bad address is refused before any request", async () => {
  start();
  render(MailPage);
  await page.getByRole("button", { name: /New message/u }).click();
  await page.getByRole("button", { name: "Send" }).click();
  await expect.element(page.getByText("Add at least one recipient.")).toBeVisible();
  await page.getByLabelText("To", { exact: true }).fill("not-an-address");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.element(page.getByText("Not a valid address: not-an-address")).toBeVisible();
  expect(native.send).not.toHaveBeenCalled();
});

test("save draft stores it through the draft command", async () => {
  start();
  native.saveDraft.mockResolvedValue(ok(null));
  render(MailPage);
  await page.getByRole("button", { name: /New message/u }).click();
  await page.getByLabelText("Subject").fill("Later");
  await page.getByRole("button", { name: "Save draft" }).click();
  expect(native.saveDraft).toHaveBeenCalledWith(
    "acc-1",
    expect.objectContaining({ subject: "Later" }),
  );
  await expect.element(page.getByText("Draft saved in your Drafts folder.")).toBeVisible();
});

test("a reply quotes the original and threads it", async () => {
  start();
  const header = message(1, { subject: "Plan", message_id: "<m1@x>" });
  native.messages.mockResolvedValue(pageOf([header]));
  native.message.mockResolvedValue(ok(body(header, { text: "Original line" })));
  native.setFlags.mockResolvedValue(ok(null));
  native.send.mockResolvedValue(ok({ sent: true, saved_copy: true }));
  render(MailPage);
  await page.getByRole("button", { name: /Plan/u }).click();
  await page.getByRole("button", { name: "Reply (R)" }).click();
  await expect.element(page.getByLabelText("Subject")).toHaveValue("Re: Plan");
  await expect
    .element(page.getByLabelText("To", { exact: true }))
    .toHaveValue("Ann Example <ann@example.org>");
  const text = page.getByRole("textbox", { name: "Message", exact: true });
  await vi.waitFor(() => {
    expect((text.element() as HTMLTextAreaElement).value).toContain("> Original line");
  });
  await page.getByRole("button", { name: "Send" }).click();
  expect(native.send.mock.calls[0]![1]).toMatchObject({
    in_reply_to: "<m1@x>",
    references: ["<m1@x>"],
  });
});

test("closing a message that has text asks before discarding it", async () => {
  start();
  render(MailPage);
  await page.getByRole("button", { name: /New message/u }).click();
  await page.getByRole("textbox", { name: "Message", exact: true }).fill("draft text");
  await page.getByRole("button", { name: "Close" }).click();
  await expect.element(page.getByText("Discard this message?")).toBeVisible();
  await page.getByRole("button", { name: "Keep editing" }).click();
  await expect
    .element(page.getByRole("textbox", { name: "Message", exact: true }))
    .toHaveValue("draft text");
});

const link = {
  to: ["ann@example.org"],
  cc: [],
  bcc: [],
  subject: "From a page",
  body: "",
};

test("a clicked mailto link waits while a message with text is open and never replaces it", async () => {
  start();
  render(MailPage);
  await page.getByRole("button", { name: /New message/u }).click();
  await page.getByLabelText("Subject").fill("My own words");
  mailtoInbox.receive(link);
  await expect.element(page.getByRole("alertdialog")).toBeVisible();
  await expect
    .element(page.getByText("A link wants to start a new message", { exact: false }))
    .toBeVisible();
  // The message being written is untouched while the question is open.
  await expect.element(page.getByLabelText("Subject")).toHaveValue("My own words");
  await page.getByRole("button", { name: "Keep writing" }).click();
  await expect.element(page.getByRole("alertdialog")).not.toBeInTheDocument();
  await expect.element(page.getByLabelText("Subject")).toHaveValue("My own words");
  expect(native.saveDraft).not.toHaveBeenCalled();
});

test("the waiting link opens after the open message was saved as a draft", async () => {
  start();
  native.saveDraft.mockResolvedValue(ok(null));
  render(MailPage);
  await page.getByRole("button", { name: /New message/u }).click();
  await page.getByLabelText("Subject").fill("My own words");
  mailtoInbox.receive(link);
  await page.getByRole("button", { name: "Save draft and open link" }).click();
  await vi.waitFor(() => expect(native.saveDraft).toHaveBeenCalledTimes(1));
  expect(native.saveDraft.mock.calls[0]![1]).toMatchObject({ subject: "My own words" });
  await expect.element(page.getByLabelText("Subject")).toHaveValue("From a page");
  await expect.element(page.getByRole("alertdialog")).not.toBeInTheDocument();
});

test("a link that arrives while the draft cannot be saved leaves the message where it is", async () => {
  start();
  native.saveDraft.mockResolvedValue(failed("offline"));
  render(MailPage);
  await page.getByRole("button", { name: /New message/u }).click();
  await page.getByLabelText("Subject").fill("My own words");
  mailtoInbox.receive(link);
  await page.getByRole("button", { name: "Save draft and open link" }).click();
  await vi.waitFor(() => expect(native.saveDraft).toHaveBeenCalledTimes(1));
  await expect.element(page.getByLabelText("Subject")).toHaveValue("My own words");
  await expect.element(page.getByRole("alertdialog")).toBeVisible();
});

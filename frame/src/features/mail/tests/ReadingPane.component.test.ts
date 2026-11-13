import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import "$styles/global.css";
import MailPage from "../components/MailPage.svelte";
import { account, body, folders, message, ok, pageOf } from "./fixtures";

const native = vi.hoisted(() => ({
  accounts: vi.fn(),
  folders: vi.fn(),
  messages: vi.fn(),
  message: vi.fn(),
  setFlags: vi.fn(),
  openUrl: vi.fn(),
  attachment: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    mailAccounts: native.accounts,
    mailFolders: native.folders,
    mailMessages: native.messages,
    mailMessage: native.message,
    mailSetFlags: native.setFlags,
    browserOpenUrl: native.openUrl,
    mailAttachment: native.attachment,
  });
});

afterEach(() => {
  for (const mock of Object.values(native)) mock.mockReset();
});

async function openWith(content: Parameters<typeof body>[1]) {
  const header = message(1);
  native.accounts.mockResolvedValue(ok([account]));
  native.folders.mockResolvedValue(ok(folders));
  native.messages.mockResolvedValue(pageOf([header]));
  native.message.mockResolvedValue(ok(body(header, content)));
  render(MailPage);
  await page.getByRole("button", { name: /Subject 1/u }).click();
}

const frame = () => document.querySelector<HTMLIFrameElement>("iframe");

test("plain text is drawn as text, never as markup", async () => {
  await openWith({ text: "<script>alert(1)</script> <b>not bold</b>" });
  await expect.element(page.getByText("<b>not bold</b>", { exact: false })).toBeVisible();
  expect(document.querySelector("iframe")).toBeNull();
  expect(document.querySelector("article script, article b")).toBeNull();
});

test("html mail is sanitised into a sandboxed frame with no script and a no-network policy", async () => {
  await openWith({
    html: '<p onclick="x()">Hi</p><script>alert(1)</script><a href="javascript:alert(1)">bad</a>',
  });
  await vi.waitFor(() => expect(frame()).not.toBeNull());
  const element = frame()!;
  expect(element.getAttribute("sandbox")).toBe("");
  const doc = element.getAttribute("srcdoc")!;
  expect(doc).toContain("default-src 'none'; img-src data:; style-src 'unsafe-inline'");
  expect(doc).toContain("<p>Hi</p>");
  expect(doc).not.toMatch(/<script|onclick|javascript:|<base/iu);
});

test("remote images stay blocked until Load images, which applies to that message", async () => {
  await openWith({ html: '<p>Hi</p><img src="https://tracker.example/p.gif" alt="t">' });
  await expect.element(page.getByText("Remote images are blocked.")).toBeVisible();
  await vi.waitFor(() => expect(frame()).not.toBeNull());
  expect(frame()!.getAttribute("srcdoc")).not.toContain("tracker.example");
  await page.getByRole("button", { name: "Load images" }).click();
  await vi.waitFor(() =>
    expect(frame()!.getAttribute("srcdoc")).toContain("https://tracker.example/p.gif"),
  );
  expect(frame()!.getAttribute("srcdoc")).toContain("img-src data: https:");
  await expect.element(page.getByText("Remote images are blocked.")).not.toBeInTheDocument();
});

test("cid images come from the message's own attachments as data urls", async () => {
  await openWith({
    html: '<img src="cid:logo@x">',
    inline_images: [
      { content_id: "<logo@x>", content_type: "image/png", data_base64: "iVBORw0KGgo=" },
    ],
  });
  await vi.waitFor(() => expect(frame()).not.toBeNull());
  expect(frame()!.getAttribute("srcdoc")).toContain('src="data:image/png;base64,iVBORw0KGgo="');
});

test("a link in plain text shows its real target and opens a tab only on confirmation", async () => {
  await openWith({ text: "see https://example.com/a?b=1 now" });
  await page.getByRole("button", { name: "https://example.com/a?b=1" }).click();
  await expect.element(page.getByText("This link goes to")).toBeVisible();
  expect(native.openUrl).not.toHaveBeenCalled();
  await page.getByRole("button", { name: "Open in new tab" }).click();
  expect(native.openUrl).toHaveBeenCalledExactlyOnceWith("https://example.com/a?b=1", true);
});

test("links in html mail are listed with their real address, and dangerous ones are not", async () => {
  await openWith({
    html:
      '<a href="https://real.example/x" title="https://trusted.example">pay</a>' +
      '<a href="javascript:alert(1)">bad</a>',
  });
  await page.getByText(/Links in this message/u).click();
  await expect.element(page.getByRole("button", { name: "https://real.example/x" })).toBeVisible();
  expect(document.body.textContent).not.toContain("trusted.example");
  expect(document.body.textContent).not.toContain("javascript:");
  await page.getByRole("button", { name: "https://real.example/x" }).click();
  await page.getByRole("button", { name: "Open in new tab" }).click();
  expect(native.openUrl).toHaveBeenCalledWith("https://real.example/x", true);
});

test("attachments are listed with their size and saved through the native dialog", async () => {
  native.attachment.mockResolvedValue(ok("saved"));
  await openWith({
    attachments: [
      {
        part: 2,
        filename: "report.pdf",
        content_type: "application/pdf",
        size: 2048,
        content_id: null,
        inline: false,
      },
    ],
  });
  await expect.element(page.getByText("report.pdf")).toBeVisible();
  await expect.element(page.getByText("2.0 KB")).toBeVisible();
  await page.getByRole("button", { name: "Save report.pdf" }).click();
  expect(native.attachment).toHaveBeenCalledWith("acc-1", "INBOX", 1, 2);
  await expect.element(page.getByText("Saved", { exact: true })).toBeVisible();
});

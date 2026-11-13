import { beforeEach, describe, expect, test, vi } from "vitest";
import * as m from "$shared/i18n/messages";
import type { MailChanged } from "$shared/ipc/bindings";
import { folder } from "./fixtures";
import {
  addArrivals,
  badgeText,
  MailUnread,
  newMailToast,
  unreadInInboxes,
} from "../unread.svelte";

const host = vi.hoisted(() => ({
  accounts: vi.fn(),
  folders: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ mailAccounts: host.accounts, mailFolders: host.folders });
});

const arrival = (account: string, count: number): MailChanged => ({
  account,
  kind: "new_mail",
  folder: "INBOX",
  unread: count,
  new_messages: count,
});

beforeEach(() => {
  host.accounts.mockReset().mockResolvedValue({ status: "ok", data: [] });
  host.folders.mockReset().mockResolvedValue({ status: "ok", data: [] });
});

describe("the badge text", () => {
  test("is nothing at zero and the count up to the cap", () => {
    expect(badgeText(0)).toBeNull();
    expect(badgeText(-1)).toBeNull();
    expect(badgeText(Number.NaN)).toBeNull();
    expect(badgeText(1)).toBe("1");
    expect(badgeText(99)).toBe("99");
  });

  test("caps above 99 with a plus", () => {
    expect(badgeText(100)).toBe("99+");
    expect(badgeText(12_000)).toBe("99+");
  });
});

describe("the unread total", () => {
  test("sums the unread of every inbox and ignores other folders", () => {
    const total = unreadInInboxes({
      a1: [folder("INBOX", "inbox", 4), folder("Archive", "archive", 30)],
      a2: [folder("INBOX", "inbox", 2), folder("Drafts", "drafts", 9)],
      a3: [],
    });
    expect(total).toBe(6);
  });

  test("is zero when no account has been read", () => {
    expect(unreadInInboxes({})).toBe(0);
  });
});

describe("arrivals", () => {
  test("only a new-mail change adds to a batch", () => {
    expect(addArrivals(0, arrival("a1", 3))).toBe(3);
    expect(addArrivals(3, arrival("a2", 2))).toBe(5);
    expect(addArrivals(5, { ...arrival("a1", 9), kind: "counts" })).toBe(5);
    expect(addArrivals(5, { ...arrival("a1", 9), kind: "removed" })).toBe(5);
  });
});

describe("the alert words", () => {
  test("a single message is said as one, a batch as its count", () => {
    expect(newMailToast(1)).toEqual({ title: m.mail_toast_title(), body: m.mail_toast_one() });
    expect(newMailToast(3)).toEqual({
      title: m.mail_toast_title(),
      body: m.mail_toast_many({ count: 3 }),
    });
    expect(newMailToast(0)).toBeNull();
  });
});

describe("MailUnread", () => {
  function store(inFront = false) {
    const alert = vi.fn();
    const unread = new MailUnread({ inFront: () => inFront, alert, batchMs: 20 });
    return { unread, alert };
  }

  test("reads the inbox counts of every account", async () => {
    host.accounts.mockResolvedValue({ status: "ok", data: [{ id: "a1" }, { id: "a2" }] });
    host.folders.mockImplementation(async (account: string) => ({
      status: "ok",
      data:
        account === "a1"
          ? [folder("INBOX", "inbox", 4)]
          : [folder("INBOX", "inbox", 1), folder("Trash", "trash", 50)],
    }));
    const { unread } = store();
    unread.receive({ ...arrival("a1", 0), kind: "accounts" });
    await vi.waitFor(() => expect(unread.total).toBe(5));
  });

  test("a failed account keeps the count it had", async () => {
    host.accounts.mockResolvedValue({ status: "ok", data: [{ id: "a1" }] });
    host.folders.mockResolvedValueOnce({ status: "ok", data: [folder("INBOX", "inbox", 7)] });
    const { unread } = store();
    unread.receive({ ...arrival("a1", 0), kind: "accounts" });
    await vi.waitFor(() => expect(unread.total).toBe(7));
    host.folders.mockResolvedValueOnce({ status: "error", error: "unavailable" });
    unread.receive({ ...arrival("a1", 0), kind: "counts" });
    await vi.waitFor(() => expect(host.folders).toHaveBeenCalledTimes(2));
    expect(unread.total).toBe(7);
  });

  test("a burst of arrivals is one alert with the total count", async () => {
    const { unread, alert } = store();
    unread.receive(arrival("a1", 2));
    unread.receive(arrival("a2", 1));
    unread.receive({ ...arrival("a1", 0), kind: "counts" });
    await vi.waitFor(() => expect(alert).toHaveBeenCalledTimes(1));
    expect(alert).toHaveBeenCalledWith(m.mail_toast_title(), m.mail_toast_many({ count: 3 }));
  });

  test("a single arrival is worded as one message", async () => {
    const { unread, alert } = store();
    unread.receive(arrival("a1", 1));
    await vi.waitFor(() => expect(alert).toHaveBeenCalledTimes(1));
    expect(alert).toHaveBeenCalledWith(m.mail_toast_title(), m.mail_toast_one());
  });

  test("no alert while the person is already on Mail in a focused window", async () => {
    const { unread, alert } = store(true);
    unread.receive(arrival("a1", 4));
    await new Promise((done) => setTimeout(done, 60));
    expect(alert).not.toHaveBeenCalled();
  });
});

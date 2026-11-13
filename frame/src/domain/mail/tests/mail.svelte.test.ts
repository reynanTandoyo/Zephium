import { afterEach, expect, test, vi } from "vitest";
import type { MailChanged, MailMessageV1 } from "$shared/ipc/bindings";
import { account, folder, message } from "./fixtures";

const host = vi.hoisted(() => ({
  changed: null as null | ((event: { payload: unknown }) => void),
  accounts: vi.fn(),
  folders: vi.fn(),
  sync: vi.fn(),
  messages: vi.fn(),
  message: vi.fn(),
  setFlags: vi.fn(),
  move: vi.fn(),
  del: vi.fn(),
  search: vi.fn(),
  removeAccount: vi.fn(),
  watch: vi.fn(),
  images: vi.fn(),
  order: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    mailAccounts: host.accounts,
    mailFolders: host.folders,
    mailSync: host.sync,
    mailMessages: host.messages,
    mailMessage: host.message,
    mailSetFlags: host.setFlags,
    mailMove: host.move,
    mailDelete: host.del,
    mailSearch: host.search,
    mailRemoveAccount: host.removeAccount,
    mailWatch: host.watch,
    mailFetchRemoteImages: host.images,
    mailSetAccountOrder: host.order,
  });
});
vi.mock("$shared/ipc/native-events", () => ({
  events: {
    mailChanged: {
      listen: (listener: (event: { payload: unknown }) => void) => {
        host.changed = listener;
        return Promise.resolve(() => {
          host.changed = null;
        });
      },
    },
  },
}));

const { MailStore } = await import("../mail.svelte");

const ok = <T>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });
const page = (messages: MailMessageV1[], next: number | null = null) =>
  ok({ messages, next_before: next, total: messages.length, from_cache: false });

afterEach(() => {
  for (const mock of Object.values(host)) {
    if (typeof mock === "function") (mock as ReturnType<typeof vi.fn>).mockReset();
  }
  host.changed = null;
});

function twoAccounts() {
  host.accounts.mockReturnValue(ok([account("a1"), account("a2")]));
  host.folders.mockImplementation((id: string) =>
    ok([
      folder("Trash", "trash"),
      folder("INBOX", "inbox", id === "a1" ? 2 : 1),
      folder("Sent", "sent"),
      folder("Work"),
    ]),
  );
  host.messages.mockImplementation((id: string) =>
    page(
      id === "a1"
        ? [message(2, { date: 50 }), message(1, { date: 10 })]
        : [message(7, { date: 30 })],
    ),
  );
}

test("start subscribes first, reads accounts and folders, and shows the combined inbox", async () => {
  twoAccounts();
  const store = new MailStore();
  await store.start();
  expect(host.changed).not.toBeNull();
  expect(store.accountsState).toBe("ready");
  expect(store.rows.map((row) => `${row.account}:${row.message.uid}`)).toEqual([
    "a1:2",
    "a2:7",
    "a1:1",
  ]);
  expect(store.unread).toBe(3);
  expect(store.foldersOf("a1").map((f) => f.role)).toEqual(["inbox", "sent", "trash", "other"]);
  store.dispose();
  expect(host.changed).toBeNull();
});

test("a failed account read is failed, and a failed list is reported with the cause", async () => {
  host.accounts.mockReturnValue(err("internal"));
  const store = new MailStore();
  await store.start();
  expect(store.accountsState).toBe("failed");
  store.dispose();

  twoAccounts();
  host.messages.mockReturnValue(err("offline"));
  const second = new MailStore();
  await second.start();
  expect(second.listState).toBe("failed");
  expect(second.listError).toBe("offline");
  second.dispose();
});

test("a request that never answers settles as a failure after its timeout", async () => {
  vi.useFakeTimers();
  host.accounts.mockReturnValue(new Promise(() => {}));
  const store = new MailStore();
  const started = store.start();
  await vi.advanceTimersByTimeAsync(31_000);
  await started;
  expect(store.accountsState).toBe("failed");
  store.dispose();
  vi.useRealTimers();
});

test("paging asks for the page before the cursor and merges without duplicates", async () => {
  host.accounts.mockReturnValue(ok([account("a1")]));
  host.folders.mockReturnValue(ok([folder("INBOX", "inbox")]));
  host.messages.mockReturnValueOnce(page([message(9, { date: 90 }), message(8, { date: 80 })], 8));
  const store = new MailStore();
  await store.start();
  expect(store.hasMore).toBe(true);
  host.messages.mockReturnValueOnce(
    page([message(8, { date: 80 }), message(7, { date: 70 })], null),
  );
  await store.loadMore();
  expect(host.messages).toHaveBeenLastCalledWith("a1", "INBOX", { before_uid: 8, limit: 50 });
  expect(store.rows.map((r) => r.message.uid)).toEqual([9, 8, 7]);
  expect(store.hasMore).toBe(false);
  store.dispose();
});

test("flags change only after the server accepted, and a refusal leaves the row alone", async () => {
  twoAccounts();
  const store = new MailStore();
  await store.start();
  const row = store.rows.find((r) => r.message.uid === 2)!;
  let release!: (value: unknown) => void;
  host.setFlags.mockReturnValueOnce(new Promise((resolve) => (release = resolve)));
  const done = store.setFlagged([row], true);
  expect(store.isPending(row.key)).toBe(true);
  expect(store.rows.find((r) => r.key === row.key)!.message.flags.flagged).toBe(false);
  release({ status: "ok", data: null });
  await done;
  expect(store.pending).toEqual([]);
  expect(store.rows.find((r) => r.key === row.key)!.message.flags.flagged).toBe(true);

  host.setFlags.mockReturnValueOnce(err("server_rejected"));
  expect(await store.setFlagged([row], false)).toBe(false);
  expect(store.rows.find((r) => r.key === row.key)!.message.flags.flagged).toBe(true);
  expect(store.notice).toEqual({ action: "flag", error: "server_rejected" });
  store.dispose();
});

test("move and delete drop rows only after success, one command per folder", async () => {
  twoAccounts();
  const store = new MailStore();
  await store.start();
  const mine = store.rows.filter((r) => r.account === "a1");
  host.move.mockReturnValueOnce(err("busy"));
  expect(await store.move(mine, "Work")).toBe(false);
  expect(store.rows).toHaveLength(3);
  host.move.mockReturnValueOnce(ok(null));
  await store.move(mine, "Work");
  expect(host.move).toHaveBeenLastCalledWith("a1", "INBOX", [2, 1], "Work");
  expect(store.rows.map((r) => r.account)).toEqual(["a2"]);
  host.del.mockReturnValueOnce(ok(null));
  await store.remove(store.rows);
  expect(host.del).toHaveBeenCalledWith("a2", "INBOX", [7]);
  expect(store.rows).toEqual([]);
  store.dispose();
});

test("archive says so when the account has no archive folder", async () => {
  host.accounts.mockReturnValue(ok([account("a1")]));
  host.folders.mockReturnValue(ok([folder("INBOX", "inbox")]));
  host.messages.mockReturnValue(page([message(1)]));
  const store = new MailStore();
  await store.start();
  expect(await store.moveToRole(store.rows, "archive")).toBe(false);
  expect(store.notice).toEqual({ action: "archive", error: "no_folder" });
  expect(host.move).not.toHaveBeenCalled();
  store.dispose();
});

test("opening marks unseen mail read only after the body arrived and the server agreed", async () => {
  host.accounts.mockReturnValue(ok([account("a1")]));
  host.folders.mockReturnValue(ok([folder("INBOX", "inbox", 1)]));
  host.messages.mockReturnValue(
    page([
      message(1, {
        flags: { seen: false, flagged: false, answered: false, draft: false, deleted: false },
      }),
    ]),
  );
  const store = new MailStore();
  await store.start();
  const row = store.rows[0]!;
  host.message.mockReturnValueOnce(err("timeout"));
  await store.openRow(row);
  expect(store.open).toMatchObject({ state: "failed", error: "timeout" });
  expect(host.setFlags).not.toHaveBeenCalled();
  host.message.mockReturnValueOnce(
    ok({
      header: row.message,
      reply_to: [],
      text: "hi",
      html: null,
      truncated: false,
      attachments: [],
      inline_images: [],
    }),
  );
  host.setFlags.mockReturnValueOnce(ok(null));
  await store.openRow(row);
  expect(store.open?.state).toBe("ready");
  await vi.waitFor(() => expect(store.rows[0]!.message.flags.seen).toBe(true));
  expect(host.setFlags).toHaveBeenCalledWith("a1", "INBOX", [1], "seen", true);
  store.dispose();
});

test("a newer selection outranks a slower earlier answer", async () => {
  twoAccounts();
  const store = new MailStore();
  await store.start();
  let slow!: (value: unknown) => void;
  host.messages.mockReturnValueOnce(new Promise((resolve) => (slow = resolve)));
  const first = store.select("a1", "INBOX");
  host.messages.mockReturnValueOnce(page([message(77, { date: 5 })]));
  await store.select("a1", "Work");
  slow({
    status: "ok",
    data: { messages: [message(1)], next_before: null, total: 1, from_cache: false },
  });
  await first;
  expect(store.rows.map((r) => r.message.uid)).toEqual([77]);
  store.dispose();
});

test("events refresh counts and the visible list; search results are left alone", async () => {
  twoAccounts();
  const store = new MailStore();
  await store.start();
  host.folders.mockClear();
  host.messages.mockClear();
  const event: MailChanged = {
    account: "a1",
    kind: "new_mail",
    folder: "INBOX",
    unread: 3,
    new_messages: 1,
  };
  host.changed!({ payload: event });
  await vi.waitFor(() => expect(host.messages).toHaveBeenCalledTimes(2));
  expect(host.folders).toHaveBeenCalledWith("a1");

  host.search.mockReturnValue(ok([message(5)]));
  await store.search("invoice");
  host.messages.mockClear();
  host.changed!({ payload: event });
  await vi.waitFor(() => expect(host.folders).toHaveBeenCalled());
  expect(host.messages).not.toHaveBeenCalled();
  expect(store.searching).toBe(true);
  expect(host.search).toHaveBeenCalledWith("a1", "INBOX", "invoice");
  store.dispose();
});

test("the accounts event rereads the list and drops folders of removed accounts", async () => {
  twoAccounts();
  const store = new MailStore();
  await store.start();
  host.accounts.mockReturnValue(ok([account("a1")]));
  host.changed!({
    payload: { account: "a2", kind: "accounts", folder: null, unread: 0, new_messages: 0 },
  });
  await vi.waitFor(() => expect(store.accounts).toHaveLength(1));
  expect(Object.keys(store.folders)).toEqual(["a1"]);
  store.dispose();
});

test("removing an account waits for the answer and keeps it on failure", async () => {
  twoAccounts();
  const store = new MailStore();
  await store.start();
  host.removeAccount.mockReturnValueOnce(err("internal"));
  expect(await store.removeAccount("a2")).toBe(false);
  expect(store.accounts).toHaveLength(2);
  expect(store.notice).toEqual({ action: "account", error: "internal" });
  store.dispose();
});

test("a reply needs an open message; a new message uses the selected account", async () => {
  twoAccounts();
  const store = new MailStore();
  await store.start();
  expect(store.startCompose("reply")).toBe(false);
  await store.select("a2", "INBOX");
  expect(store.startCompose("new")).toBe(true);
  expect(store.compose?.account).toBe("a2");
  store.closeCompose();
  expect(store.compose).toBeNull();
  store.dispose();
});

async function openFirst() {
  twoAccounts();
  host.message.mockImplementation((_id: string, _folder: string, uid: number) =>
    ok({
      header: message(uid),
      reply_to: [],
      text: null,
      html: '<img src="https://img.example/a.png">',
      truncated: false,
      attachments: [],
      inline_images: [],
    }),
  );
  const store = new MailStore();
  await store.start();
  await store.openRow(store.rows[0]!);
  return store;
}

const pixel = "data:image/png;base64,iVBORw0KGgo=";

test("remote images are asked for once per message and arrive as data urls", async () => {
  const store = await openFirst();
  expect(store.open?.remote.state).toBe("idle");
  host.images.mockReturnValue(ok([{ url: "https://img.example/a.png", data_url: pixel }]));
  expect(
    await store.loadRemoteImages(["https://img.example/a.png", "https://img.example/a.png"]),
  ).toBe(true);
  expect(host.images).toHaveBeenCalledExactlyOnceWith("a1", "INBOX", 2, [
    "https://img.example/a.png",
  ]);
  expect(store.open?.remote.state).toBe("ready");
  expect(store.open?.remote.images["https://img.example/a.png"]).toBe(pixel);
  store.dispose();
});

test("at most 24 images are requested, and nothing is asked when there are none", async () => {
  const store = await openFirst();
  host.images.mockReturnValue(ok([]));
  expect(await store.loadRemoteImages([])).toBe(false);
  expect(host.images).not.toHaveBeenCalled();
  const many = Array.from({ length: 40 }, (_, n) => `https://img.example/${n}.png`);
  await store.loadRemoteImages(many);
  expect(host.images.mock.calls[0]![3]).toHaveLength(24);
  store.dispose();
});

test("a failed image fetch is recorded with its cause and can be retried", async () => {
  const store = await openFirst();
  host.images.mockReturnValueOnce(err("offline"));
  expect(await store.loadRemoteImages(["https://img.example/a.png"])).toBe(false);
  expect(store.open?.remote).toMatchObject({ state: "failed", error: "offline" });
  host.images.mockReturnValueOnce(ok([{ url: "https://img.example/a.png", data_url: pixel }]));
  expect(await store.loadRemoteImages(["https://img.example/a.png"])).toBe(true);
  expect(store.open?.remote.state).toBe("ready");
  store.dispose();
});

test("images that arrive after another message was opened are dropped", async () => {
  const store = await openFirst();
  let release: (value: unknown) => void = () => {};
  host.images.mockReturnValue(new Promise((resolve) => (release = resolve)));
  const pending = store.loadRemoteImages(["https://img.example/a.png"]);
  await store.openRow(store.rows[1]!);
  release(ok([{ url: "https://img.example/a.png", data_url: pixel }]));
  expect(await pending).toBe(false);
  expect(store.open?.row.key).toBe(store.rows[1]!.key);
  expect(store.open?.remote.state).toBe("idle");
  store.dispose();
});

test("moving an account saves the whole order first and the list follows the answer", async () => {
  host.accounts.mockReturnValue(ok([account("a1"), account("a2"), account("a3")]));
  host.folders.mockReturnValue(ok([folder("INBOX", "inbox")]));
  host.messages.mockReturnValue(page([]));
  const store = new MailStore();
  await store.start();
  host.order.mockReturnValue(ok(null));
  expect(await store.moveAccount("a3", -1)).toBe(true);
  expect(host.order).toHaveBeenCalledWith(["a1", "a3", "a2"]);
  expect(store.accounts.map((a) => a.id)).toEqual(["a1", "a3", "a2"]);
  expect(await store.moveAccount("a1", -1)).toBe(false);
  expect(await store.moveAccount("a2", 1)).toBe(false);
  expect(await store.moveAccount("nope", 1)).toBe(false);
  expect(host.order).toHaveBeenCalledTimes(1);
  store.dispose();
});

test("a refused reorder leaves the list as it was and says so", async () => {
  host.accounts.mockReturnValue(ok([account("a1"), account("a2")]));
  host.folders.mockReturnValue(ok([folder("INBOX", "inbox")]));
  host.messages.mockReturnValue(page([]));
  const store = new MailStore();
  await store.start();
  host.order.mockReturnValue(err("busy"));
  expect(await store.moveAccount("a2", -1)).toBe(false);
  expect(store.accounts.map((a) => a.id)).toEqual(["a1", "a2"]);
  expect(store.notice).toEqual({ action: "order", error: "busy" });
  store.dispose();
});

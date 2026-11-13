import { afterEach, expect, test, vi } from "vitest";

const host = vi.hoisted(() => ({
  compose: null as null | ((event: { payload: unknown }) => void),
  stops: 0,
}));
vi.mock("$shared/ipc/native-events", () => ({
  events: {
    mailCompose: {
      listen: (listener: (event: { payload: unknown }) => void) => {
        host.compose = listener;
        return Promise.resolve(() => {
          host.compose = null;
          host.stops++;
        });
      },
    },
  },
}));

const { MailtoInbox } = await import("../mailto.svelte");

const link = { to: ["ann@example.org"], cc: [], bcc: [], subject: "Hi", body: "" };

afterEach(() => {
  host.compose = null;
  host.stops = 0;
});

test("a clicked link waits in the inbox until it is taken, once", async () => {
  const inbox = new MailtoInbox();
  const release = inbox.hold();
  await vi.waitFor(() => expect(host.compose).not.toBeNull());
  host.compose!({ payload: { mailto: link } });
  expect(inbox.pending).toEqual(link);
  expect(inbox.take()).toEqual(link);
  expect(inbox.take()).toBeNull();
  release();
});

test("the first holder listens and the last one stops, dropping what was never taken", async () => {
  const inbox = new MailtoInbox();
  const one = inbox.hold();
  const two = inbox.hold();
  await vi.waitFor(() => expect(host.compose).not.toBeNull());
  host.compose!({ payload: { mailto: link } });
  one();
  one();
  expect(host.compose).not.toBeNull();
  two();
  expect(host.stops).toBe(1);
  expect(inbox.pending).toBeNull();
});

test("releasing before the subscription lands still stops it", async () => {
  const inbox = new MailtoInbox();
  inbox.hold()();
  await vi.waitFor(() => expect(host.stops).toBe(1));
  expect(host.compose).toBeNull();
});

test("a newer link replaces an older one that was never opened", () => {
  const inbox = new MailtoInbox();
  inbox.receive(link);
  inbox.receive({ ...link, subject: "Newer" });
  expect(inbox.take()?.subject).toBe("Newer");
});

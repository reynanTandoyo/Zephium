import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type {
  CalendarAccountV1,
  CalendarError,
  CalendarOccurrenceV1,
  CalendarV1,
} from "../../../shared/ipc/bindings";

const host = vi.hoisted(() => ({
  changed: null as null | (() => void),
  accounts: vi.fn(),
  calendars: vi.fn(),
  events: vi.fn(),
  setVisible: vi.fn(),
  create: vi.fn(),
  signIn: vi.fn(),
  addOauth: vi.fn(),
  discard: vi.fn(),
  cancel: vi.fn(),
  updateOccurrence: vi.fn(),
  deleteOccurrence: vi.fn(),
  sync: vi.fn(),
  open: vi.fn(),
}));

vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    calendarAccounts: host.accounts,
    calendarCalendars: host.calendars,
    calendarEvents: host.events,
    calendarSetVisible: host.setVisible,
    calendarCreateEvent: host.create,
    oauthSignIn: host.signIn,
    calendarAddOauthAccount: host.addOauth,
    oauthDiscardGrant: host.discard,
    oauthCancelSignIn: host.cancel,
    calendarUpdateOccurrence: host.updateOccurrence,
    calendarDeleteOccurrence: host.deleteOccurrence,
    calendarSync: host.sync,
    browserOpenUrl: host.open,
  });
});
vi.mock("$shared/ipc/native-events", () => ({
  events: {
    calendarChanged: {
      listen: (listener: () => void) => {
        host.changed = listener;
        return Promise.resolve(() => {
          host.changed = null;
        });
      },
    },
  },
}));

const { init, dispose, setRange, refresh, ...store } = await import("../calendar.svelte");

const ok = <T>(data: T) => ({ status: "ok" as const, data });
const err = (error: CalendarError) => ({ status: "error" as const, error });

const account: CalendarAccountV1 = {
  id: "a",
  kind: "caldav",
  name: "Work",
  server_url: "https://cal.example.com",
  username: "me",
};
const writable: CalendarV1 = {
  id: "c1",
  account_id: "a",
  url: "u",
  name: "Work",
  color: "#112233",
  read_only: false,
  visible: true,
};
const readOnly: CalendarV1 = { ...writable, id: "c2", read_only: true };
const occurrence = (uid: string): CalendarOccurrenceV1 => ({
  calendar_id: "c1",
  uid,
  recurrence_id: null,
  summary: uid,
  description: "",
  location: "",
  start: "2026-10-07T09:00:00",
  end: "2026-10-07T10:00:00",
  all_day: false,
  tzid: null,
  color: null,
  status: "confirmed",
  recurring: false,
  read_only: false,
  url: null,
  organizer: null,
  attendees: [],
  reminders: [],
});

const range = ["2026-10-04T22:00:00.000Z", "2026-10-11T22:00:00.000Z"] as const;

beforeEach(() => {
  host.accounts.mockResolvedValue(ok([account]));
  host.calendars.mockResolvedValue(ok([writable, readOnly]));
  host.events.mockResolvedValue(ok({ occurrences: [occurrence("a")], truncated: false }));
});
afterEach(() => {
  dispose();
  vi.clearAllMocks();
});

test("loads accounts and calendars on init and events for the range", async () => {
  await init();
  expect(store.accounts()).toEqual([account]);
  expect(store.calendars()).toHaveLength(2);
  expect(store.writableCalendars().map((entry) => entry.id)).toEqual(["c1"]);
  expect(store.listState()).toBe("ready");
  expect(store.eventState()).toBe("idle");

  await setRange(...range);
  expect(host.events).toHaveBeenCalledWith(...range);
  expect(store.occurrences().map((entry) => entry.uid)).toEqual(["a"]);
  expect(store.eventState()).toBe("ready");
});

test("does not read the same range twice", async () => {
  await init();
  await setRange(...range);
  await setRange(...range);
  expect(host.events).toHaveBeenCalledTimes(1);
});

test("reads again when native says calendars changed", async () => {
  await init();
  await setRange(...range);
  host.events.mockResolvedValue(ok({ occurrences: [occurrence("b")], truncated: true }));
  host.changed!();
  await vi.waitFor(() => expect(store.occurrences().map((entry) => entry.uid)).toEqual(["b"]));
  expect(store.truncated()).toBe(true);
});

test("a failed read is a failed state with its reason, and the old events stay until a read succeeds", async () => {
  await init();
  await setRange(...range);
  host.events.mockResolvedValue(err("network"));
  await refresh();
  expect(store.eventState()).toBe("failed");
  expect(store.eventFault()).toBe("network");
  expect(store.occurrences()).toHaveLength(1);
  host.events.mockResolvedValue(ok({ occurrences: [], truncated: false }));
  await store.retry();
  expect(store.eventState()).toBe("ready");
  expect(store.eventFault()).toBeNull();
});

test("a thrown command is reported as unavailable, not as success", async () => {
  host.accounts.mockRejectedValue(new Error("no host"));
  await init();
  expect(store.listState()).toBe("failed");
  expect(store.listFault()).toBe("unavailable");
});

test("a newer read outranks a slower older one", async () => {
  await init();
  let release!: (value: unknown) => void;
  host.events.mockImplementationOnce(() => new Promise((resolve) => (release = resolve)));
  const slow = setRange("2026-01-01T00:00:00.000Z", "2026-01-08T00:00:00.000Z");
  host.events.mockResolvedValueOnce(ok({ occurrences: [occurrence("new")], truncated: false }));
  await setRange(...range);
  release(ok({ occurrences: [occurrence("old")], truncated: false }));
  await slow;
  expect(store.occurrences().map((entry) => entry.uid)).toEqual(["new"]);
});

test("a refused change shows nothing as done and does not read back", async () => {
  await init();
  await setRange(...range);
  host.create.mockResolvedValue(err("read_only"));
  const callsBefore = host.events.mock.calls.length;
  const result = await store.createEvent("c2", {
    summary: "x",
    description: "",
    location: "",
    start: "2026-10-07T09:00:00",
    end: "2026-10-07T10:00:00",
    all_day: false,
    tzid: "Europe/Berlin",
    rrule: null,
    reminders: [],
    url: null,
  });
  expect(result).toEqual({ ok: false, error: "read_only" });
  expect(host.events.mock.calls.length).toBe(callsBefore);
  expect(store.busy()).toBeNull();
});

test("an admitted change is read back before it is shown", async () => {
  await init();
  await setRange(...range);
  host.setVisible.mockResolvedValue(ok(null));
  host.events.mockResolvedValue(ok({ occurrences: [], truncated: false }));
  const result = await store.setVisible("c1", false);
  expect(result.ok).toBe(true);
  expect(host.setVisible).toHaveBeenCalledWith("c1", false);
  expect(store.occurrences()).toEqual([]);
});

test("one operation at a time", async () => {
  await init();
  let release!: (value: unknown) => void;
  host.sync.mockImplementationOnce(() => new Promise((resolve) => (release = resolve)));
  const first = store.sync();
  expect(store.busy()).toBe("sync");
  const second = await store.sync();
  expect(second).toEqual({ ok: false, error: "unavailable" });
  release(ok({ outcomes: [{ calendar_id: "c1", error: "network" }] }));
  await first;
  expect(store.busy()).toBeNull();
  expect(store.syncFailures()).toEqual(["c1"]);
});

test("opening a link reports whether native accepted it", async () => {
  host.open.mockResolvedValueOnce({ accepted: true, operation_id: null });
  expect(await store.openLink("https://meet.google.com/abc-defg-hij")).toBe(true);
  expect(host.open).toHaveBeenCalledWith("https://meet.google.com/abc-defg-hij", true);
  host.open.mockRejectedValueOnce(new Error("no"));
  expect(await store.openLink("https://meet.google.com/abc-defg-hij")).toBe(false);
});

test("disposal stops listening and drops what was loaded", async () => {
  await init();
  await setRange(...range);
  dispose();
  expect(host.changed).toBeNull();
  expect(store.accounts()).toEqual([]);
  expect(store.eventState()).toBe("idle");
});

test("changes and deletes one occurrence by its recurrence id, then reads back", async () => {
  await init();
  await setRange(...range);
  host.updateOccurrence.mockResolvedValue(ok({ calendar_id: "c1", uid: "a" }));
  host.deleteOccurrence.mockResolvedValue(ok(null));
  const draft = {
    summary: "x",
    description: "",
    location: "",
    start: "2026-10-14T09:00:00",
    end: "2026-10-14T10:00:00",
    all_day: false,
    tzid: "Europe/Berlin",
    rrule: null,
    reminders: [],
    url: null,
  };
  const before = host.events.mock.calls.length;
  expect((await store.updateOccurrence("c1", "a", "2026-10-14T09:00:00", draft)).ok).toBe(true);
  expect(host.updateOccurrence).toHaveBeenCalledWith("c1", "a", "2026-10-14T09:00:00", draft);
  expect((await store.deleteOccurrence("c1", "a", "2026-10-14T09:00:00")).ok).toBe(true);
  expect(host.deleteOccurrence).toHaveBeenCalledWith("c1", "a", "2026-10-14T09:00:00");
  expect(host.events.mock.calls.length).toBe(before + 2);
  host.deleteOccurrence.mockResolvedValue(err("not_found"));
  expect(await store.deleteOccurrence("c1", "a", "2026-10-21T09:00:00")).toEqual({
    ok: false,
    error: "not_found",
  });
});

const grant = {
  handle: "h1",
  provider: "google",
  client_id: "id",
  tenant: null,
  expires_in_secs: 600,
};

test("signs in for calendar, adds the account and reads back", async () => {
  await init();
  host.signIn.mockResolvedValue(ok(grant));
  host.addOauth.mockResolvedValue(ok({ ...account, kind: "google" }));
  const before = host.accounts.mock.calls.length;
  const result = await store.addOauthAccount("google", " id ", "ignored", "s3cret", " Mine ");
  expect(result).toEqual({ ok: true });
  // Google sends its secret and no tenant; the purpose is calendar only.
  expect(host.signIn).toHaveBeenCalledWith("google", "id", null, "s3cret", ["calendar"]);
  expect(host.addOauth).toHaveBeenCalledWith("h1", "Mine");
  expect(host.discard).not.toHaveBeenCalled();
  expect(host.accounts.mock.calls.length).toBe(before + 1);
  expect(store.busy()).toBeNull();
});

test("Microsoft sends its tenant, or none for the default, and no secret", async () => {
  await init();
  host.signIn.mockResolvedValue(ok({ ...grant, provider: "microsoft" }));
  host.addOauth.mockResolvedValue(ok({ ...account, kind: "microsoft" }));
  await store.addOauthAccount("microsoft", "id", " organizations ", "ignored", "");
  expect(host.signIn).toHaveBeenLastCalledWith("microsoft", "id", "organizations", null, [
    "calendar",
  ]);
  expect(host.addOauth).toHaveBeenLastCalledWith("h1", null);
  await store.addOauthAccount("microsoft", "id", "  ", "", "");
  expect(host.signIn).toHaveBeenLastCalledWith("microsoft", "id", null, null, ["calendar"]);
});

test("a grant the backend refuses is discarded, and the reason is kept", async () => {
  await init();
  host.signIn.mockResolvedValue(ok(grant));
  host.addOauth.mockResolvedValue(err("unsupported"));
  host.discard.mockRejectedValue(new Error("gone"));
  const result = await store.addOauthAccount("google", "id", "", "", "");
  expect(result).toEqual({ ok: false, error: "unsupported" });
  expect(host.discard).toHaveBeenCalledWith("h1");
  expect(store.busy()).toBeNull();
});

test("a failed or cancelled sign-in adds nothing", async () => {
  await init();
  host.signIn.mockResolvedValue({ status: "error" as const, error: "cancelled" });
  expect(await store.addOauthAccount("google", "id", "", "", "")).toEqual({
    ok: false,
    error: "cancelled",
  });
  expect(host.addOauth).not.toHaveBeenCalled();
  host.signIn.mockRejectedValue(new Error("no host"));
  expect(await store.addOauthAccount("google", "id", "", "", "")).toEqual({
    ok: false,
    error: "unavailable",
  });
});

test("a sign-in waits for none other, and can be cancelled", async () => {
  await init();
  let release!: (value: unknown) => void;
  host.signIn.mockImplementationOnce(() => new Promise((resolve) => (release = resolve)));
  const first = store.addOauthAccount("google", "id", "", "", "");
  expect(store.busy()).toBe("account");
  expect(await store.addOauthAccount("google", "id", "", "", "")).toEqual({
    ok: false,
    error: "busy",
  });
  host.cancel.mockResolvedValue(ok(null));
  store.cancelSignIn();
  expect(host.cancel).toHaveBeenCalledTimes(1);
  release({ status: "error", error: "cancelled" });
  await first;
  expect(store.busy()).toBeNull();
});

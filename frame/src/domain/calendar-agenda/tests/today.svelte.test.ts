import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { CalendarOccurrenceV1 } from "../../../shared/ipc/bindings";

const host = vi.hoisted(() => ({
  changed: null as null | (() => void),
  accounts: vi.fn(),
  events: vi.fn(),
}));

vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ calendarAccounts: host.accounts, calendarEvents: host.events });
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
vi.mock("$domain/surface", () => ({ surface: { open: vi.fn() } }));

const today = await import("../today.svelte");

const ok = <T>(data: T) => ({ status: "ok" as const, data });
const NOW = new Date(2026, 9, 7, 10, 0, 0);

const iso = (hour: number, minute = 0) =>
  `2026-10-07T${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}:00`;

const occurrence = (patch: Partial<CalendarOccurrenceV1>): CalendarOccurrenceV1 => ({
  calendar_id: "c",
  uid: "u",
  recurrence_id: null,
  summary: "Event",
  description: "",
  location: "",
  start: iso(11),
  end: iso(12),
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
  ...patch,
});

const events = [
  occurrence({ uid: "past", summary: "Past", start: iso(8), end: iso(9) }),
  occurrence({ uid: "later", summary: "Later", start: iso(15), end: iso(16) }),
  occurrence({
    uid: "soon",
    summary: "Soon",
    start: iso(10, 5),
    end: iso(10, 45),
    location: "https://meet.google.com/abc-defg-hij",
  }),
  occurrence({
    uid: "cancelled",
    summary: "Cancelled",
    start: iso(12),
    end: iso(13),
    status: "cancelled",
  }),
  occurrence({
    uid: "all",
    summary: "Holiday",
    all_day: true,
    start: "2026-10-07",
    end: "2026-10-08",
  }),
  occurrence({ uid: "on", summary: "Running", start: iso(9, 30), end: iso(10, 30) }),
];

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
  vi.setSystemTime(NOW);
  host.accounts.mockResolvedValue(ok([{ id: "a" }]));
  host.events.mockResolvedValue(ok({ occurrences: events, truncated: false }));
});
afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});

const loaded = async () => {
  const release = today.acquire();
  await vi.waitFor(() => expect(today.available()).toBe(true));
  return release;
};

test("lists what is left of today: all-day first, then by start, without ended or cancelled", async () => {
  const release = await loaded();
  expect(today.upcoming().map((event) => event.occurrence.summary)).toEqual([
    "Holiday",
    "Running",
    "Soon",
    "Later",
  ]);
  const [from, to] = host.events.mock.calls[0] as [string, string];
  expect(new Date(to).getTime() - new Date(from).getTime()).toBe(86_400_000);
  release();
});

test("offers Join to a meeting that starts within ten minutes or is on now", async () => {
  const release = await loaded();
  const [, , soon, later] = today.upcoming();
  expect(today.joinable(soon!)?.url).toBe("https://meet.google.com/abc-defg-hij");
  expect(today.joinable(later!)).toBeNull();
  vi.setSystemTime(new Date(2026, 9, 7, 9, 54));
  await vi.advanceTimersByTimeAsync(30_000);
  expect(today.joinable(soon!)).toBeNull();
  vi.setSystemTime(new Date(2026, 9, 7, 9, 56));
  await vi.advanceTimersByTimeAsync(30_000);
  expect(today.joinable(soon!)).not.toBeNull();
  release();
});

test("drops an event once it has ended as the clock moves", async () => {
  const release = await loaded();
  vi.setSystemTime(new Date(2026, 9, 7, 10, 50));
  await vi.advanceTimersByTimeAsync(30_000);
  expect(today.upcoming().map((event) => event.occurrence.summary)).toEqual(["Holiday", "Later"]);
  release();
});

test("is not available without an account", async () => {
  host.accounts.mockResolvedValue(ok([]));
  const release = today.acquire();
  await vi.waitFor(() => expect(host.accounts).toHaveBeenCalled());
  await vi.advanceTimersByTimeAsync(0);
  expect(today.available()).toBe(false);
  release();
});

test("reads again when calendars change, and surfaces share one read", async () => {
  const release = await loaded();
  const second = today.acquire();
  expect(host.accounts).toHaveBeenCalledTimes(1);
  host.events.mockResolvedValue(ok({ occurrences: [events[1]!], truncated: false }));
  host.changed!();
  await vi.waitFor(() =>
    expect(today.upcoming().map((event) => event.occurrence.summary)).toEqual(["Later"]),
  );
  release();
  expect(host.changed).not.toBeNull();
  second();
  expect(host.changed).toBeNull();
  expect(today.available()).toBe(false);
});

test("a failed read keeps what was shown", async () => {
  const release = await loaded();
  host.events.mockRejectedValue(new Error("no host"));
  host.changed!();
  await vi.advanceTimersByTimeAsync(0);
  expect(today.upcoming()).toHaveLength(4);
  release();
});

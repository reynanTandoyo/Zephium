import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-svelte";
import "$styles/global.css";
import type { CalendarAccountV1, CalendarOccurrenceV1 } from "$shared/ipc/bindings";
import { eventOpen, linkOf } from "$domain/calendar-link";
import TodayEvents from "../components/TodayEvents.svelte";

const native = vi.hoisted(() => ({
  accounts: [] as { id: string }[],
  events: [] as CalendarOccurrenceV1[],
  run: vi.fn(async (_id: string) => ({ accepted: true, operation_id: null })),
  open: vi.fn(async (_url: string, _newTab: boolean) => ({ accepted: true, operation_id: null })),
}));

vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    calendarAccounts: async () => ({
      status: "ok" as const,
      data: native.accounts as CalendarAccountV1[],
    }),
    calendarEvents: async () => ({
      status: "ok" as const,
      data: { occurrences: [...native.events], truncated: false },
    }),
    runCommand: native.run,
    browserOpenUrl: native.open,
  });
});

const local = (minutes: number) => {
  const date = new Date(Date.now() + minutes * 60_000);
  const part = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${part(date.getMonth() + 1)}-${part(date.getDate())}T${part(date.getHours())}:${part(date.getMinutes())}:00`;
};

const event = (patch: Partial<CalendarOccurrenceV1>): CalendarOccurrenceV1 => ({
  calendar_id: "work",
  uid: "u",
  recurrence_id: null,
  summary: "",
  description: "",
  location: "",
  start: local(120),
  end: local(180),
  all_day: false,
  tzid: null,
  color: "#3366ff",
  status: "confirmed",
  recurring: false,
  read_only: false,
  url: null,
  organizer: null,
  attendees: [],
  reminders: [],
  ...patch,
});

const call = event({
  uid: "call",
  summary: "Customer call",
  start: local(5),
  end: local(35),
  location: "https://us02web.zoom.us/j/123456789?pwd=abc",
});
const review = event({
  uid: "review",
  summary: "Design review",
  start: local(120),
  end: local(150),
});

beforeEach(() => {
  native.accounts = [{ id: "acc" }];
  native.events = [review, call];
});
afterEach(() => {
  eventOpen.clear();
  vi.clearAllMocks();
});

test("lists what is left of today, soonest first, and opens Calendar on the one clicked", async () => {
  await page.viewport(360, 600);
  const screen = await render(TodayEvents);
  const rows = screen.getByRole("button", { name: /Open .* in Calendar/u });
  await expect.element(rows.first()).toHaveAccessibleName("Open Customer call in Calendar");
  await expect.element(rows.nth(1)).toHaveAccessibleName("Open Design review in Calendar");

  await rows.nth(1).click();
  expect(native.run).toHaveBeenCalledWith("browser.calendar");
  expect(eventOpen.pending()).toEqual(linkOf(review));
});

test("offers Join only to a meeting that starts within ten minutes, as a new tab", async () => {
  const screen = await render(TodayEvents);
  const join = screen.getByRole("button", { name: "Join" });
  await expect.element(join).toBeVisible();
  await join.click();
  expect(native.open).toHaveBeenCalledWith("https://us02web.zoom.us/j/123456789?pwd=abc", true);
});

test("does not offer Join to a meeting that is further away", async () => {
  native.events = [{ ...call, start: local(40), end: local(70) }];
  const screen = await render(TodayEvents);
  await expect.element(screen.getByText("Customer call")).toBeVisible();
  await expect.element(screen.getByRole("button", { name: "Join" })).not.toBeInTheDocument();
});

test("says nothing more is on, and is absent without an account", async () => {
  native.events = [];
  const screen = await render(TodayEvents);
  await expect.element(screen.getByText("Nothing more today")).toBeVisible();
  screen.unmount();

  native.accounts = [];
  const empty = await render(TodayEvents);
  await expect.element(empty.getByText("Today’s events")).not.toBeInTheDocument();
});

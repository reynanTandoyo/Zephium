import { afterEach, expect, test, vi } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-svelte";
import "$styles/global.css";
import type { CalendarAccountV1, CalendarOccurrenceV1 } from "$shared/ipc/bindings";
import { resourceTestServer } from "$shared/testing/resources/server";
import { eventOpen, linkOf } from "$domain/calendar-link";
import { preferences } from "$domain/preferences";
import { loadNewTab } from "$features/newtab";
import { loadNewTabSearch } from "$features/search";
import NewTabLook from "./NewTabLook.svelte";

const native = vi.hoisted(() => ({
  settings: {} as Record<string, string>,
  profile: "00000000000000000000000601",
  accounts: [] as { id: string }[],
  events: [] as CalendarOccurrenceV1[],
  run: vi.fn(async (_id: string) => ({ accepted: true, operation_id: null })),
  call: vi.fn(),
}));

vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    resourceCall: native.call,
    searchShortcutDefaults: async () => [],
    faviconProbe: async () => true,
    settingGet: async (key: string) => native.settings[key] ?? null,
    newtabSearchContext: async () => ({
      window_id: "window",
      profile_id: native.profile,
      space_id: "space",
      session_id: "newtab:tab:1",
      request_id: "",
    }),
    newtabSearch: async () => true,
    newtabRun: async () => ({ accepted: true, operation_id: null }),
    newtabCancel: async () => true,
    timeCall: async () => ({ kind: "focus_days" as const, days: [] }),
    blockerStats: async () => ({
      status: "ok" as const,
      data: { today: 0, last7Days: 0, days: [0, 0, 0, 0, 0, 0, 0] },
    }),
    calendarAccounts: async () => ({
      status: "ok" as const,
      data: native.accounts as CalendarAccountV1[],
    }),
    calendarEvents: async () => ({
      status: "ok" as const,
      data: { occurrences: [...native.events], truncated: false },
    }),
    runCommand: native.run,
    captionReplay: async () => {},
  });
});
vi.mock("$domain/tabs", () => ({
  tabs: { profile: () => ({ id: native.profile, name: "Alex", kind: "default" }) },
}));

await Promise.all([loadNewTab(), loadNewTabSearch()]);

const at = (minutes: number) => {
  const date = new Date(Date.now() + minutes * 60_000);
  const part = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${part(date.getMonth() + 1)}-${part(date.getDate())}T${part(date.getHours())}:${part(date.getMinutes())}:00`;
};

const planning: CalendarOccurrenceV1 = {
  calendar_id: "work",
  uid: "planning",
  recurrence_id: null,
  summary: "Planning",
  description: "",
  location: "",
  start: at(90),
  end: at(150),
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
};

afterEach(() => {
  preferences.dispose();
  eventOpen.clear();
  vi.clearAllMocks();
});

async function setup(settings: Record<string, string> = {}) {
  native.settings = settings;
  native.profile = String(BigInt(native.profile) + 1n).padStart(26, "0");
  native.call.mockImplementation(resourceTestServer(native.profile).call);
  await preferences.init();
  await page.viewport(1320, 860);
  const screen = await render(NewTabLook, {});
  screen.container.style.cssText = "inline-size:1200px;block-size:780px;padding:6px";
  // The day's figures stand once the sky has.
  await expect
    .poll(() => screen.container.querySelector('.newtab[data-ready="true"]'), { timeout: 8000 })
    .toBeTruthy();
  return screen;
}

test("shows the next event of the day and opens Calendar on it", async () => {
  native.accounts = [{ id: "acc" }];
  native.events = [planning];
  const screen = await setup();
  const figure = screen.getByRole("button", { name: /Next event .*Planning/u });
  await expect.element(figure).toBeVisible();
  await figure.click();
  expect(native.run).toHaveBeenCalledWith("browser.calendar");
  expect(eventOpen.pending()).toEqual(linkOf(planning));
});

test("says nothing more is on when the day has no events left", async () => {
  native.accounts = [{ id: "acc" }];
  native.events = [];
  const screen = await setup();
  await expect
    .element(screen.getByRole("button", { name: /Next event .*Nothing more today/u }))
    .toBeVisible();
});

test("is not there without an account", async () => {
  native.accounts = [];
  native.events = [planning];
  const screen = await setup();
  await expect.element(screen.getByRole("button", { name: /Next event/u })).not.toBeInTheDocument();
});

test("is not there with Calendar off", async () => {
  native.accounts = [{ id: "acc" }];
  native.events = [planning];
  const screen = await setup({ "calendar.enabled": "false" });
  await expect.element(screen.getByRole("button", { name: /Next event/u })).not.toBeInTheDocument();
});

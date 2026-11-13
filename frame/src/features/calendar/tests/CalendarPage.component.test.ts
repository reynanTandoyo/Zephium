import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-svelte";
import "$styles/global.css";
import type {
  CalendarAccountV1,
  CalendarError,
  CalendarEventDraftV1,
  CalendarOccurrenceV1,
  CalendarV1,
} from "$shared/ipc/bindings";
import { emitNativeEvent } from "$shared/testing/native-events";
import { calendar, dayKey, eventOpen, linkOf, parseEventUrl, startOfDay } from "$domain/calendar";
import CalendarPage from "../components/CalendarPage.svelte";

type Server = {
  accounts: CalendarAccountV1[];
  calendars: CalendarV1[];
  events: CalendarOccurrenceV1[];
  next: number;
  fail: CalendarError | null;
  discoverFails: boolean;
};

const native = vi.hoisted(() => ({
  server: null as unknown as Server,
  create: vi.fn(),
  update: vi.fn(),
  updateOccurrence: vi.fn(),
  removeOccurrence: vi.fn(),
  remove: vi.fn(),
  addAccount: vi.fn(),
  open: vi.fn(),
  createTask: vi.fn(),
}));

const ok = <T>(data: T) => ({ status: "ok" as const, data });
const err = (error: CalendarError) => ({ status: "error" as const, error });

vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    calendarAccounts: async () => ok(native.server.accounts),
    calendarCalendars: async () => ok(native.server.calendars),
    calendarEvents: async () => ok({ occurrences: [...native.server.events], truncated: false }),
    calendarSetVisible: async () => ok(null),
    calendarSync: async () => ok({ outcomes: [] }),
    calendarDiscover: async () =>
      native.server.discoverFails
        ? err("not_found")
        : ok({ server_url: "https://cal.example.com/dav", username: "me@example.com" }),
    calendarAddAccount: native.addAccount,
    calendarCreateEvent: native.create,
    calendarUpdateEvent: native.update,
    calendarUpdateOccurrence: native.updateOccurrence,
    calendarDeleteOccurrence: native.removeOccurrence,
    calendarDeleteEvent: native.remove,
    browserOpenUrl: native.open,
    runCommand: async () => ({ accepted: true, operation_id: null }),
  });
});

vi.mock("$domain/resources", () => ({
  taskSession: () => ({ create: native.createTask }),
}));
vi.mock("$domain/tabs", () => ({
  tabs: { profile: () => ({ id: "00000000000000000000000701", name: "Alex", kind: "default" }) },
}));

const today = startOfDay(new Date());
const at = (time: string) => `${dayKey(today)}T${time}`;

const occurrence = (patch: Partial<CalendarOccurrenceV1>): CalendarOccurrenceV1 => ({
  calendar_id: "work",
  uid: "uid",
  recurrence_id: null,
  summary: "",
  description: "",
  location: "",
  start: at("10:00:00"),
  end: at("11:00:00"),
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

function freshServer(): Server {
  return {
    accounts: [
      {
        id: "acc",
        kind: "caldav",
        name: "Work",
        server_url: "https://cal.example.com",
        username: "me",
      },
      {
        id: "feed",
        kind: "subscription",
        name: "Holidays",
        server_url: "https://h.example.com/h.ics",
        username: null,
      },
    ],
    calendars: [
      {
        id: "work",
        account_id: "acc",
        url: "u1",
        name: "Work",
        color: "#3366ff",
        read_only: false,
        visible: true,
      },
      {
        id: "holidays",
        account_id: "feed",
        url: "u2",
        name: "Holidays",
        color: "#cc3333",
        read_only: true,
        visible: true,
      },
    ],
    events: [
      occurrence({ uid: "standup", summary: "Standup" }),
      occurrence({
        uid: "holiday",
        calendar_id: "holidays",
        summary: "Founders Day",
        color: "#cc3333",
        all_day: true,
        start: dayKey(today),
        end: dayKey(new Date(today.getFullYear(), today.getMonth(), today.getDate() + 1)),
        read_only: true,
      }),
      occurrence({
        uid: "series",
        summary: "Weekly sync",
        start: at("14:00:00"),
        end: at("15:00:00"),
        recurring: true,
        recurrence_id: at("14:00:00"),
        series_start: `${dayKey(new Date(today.getFullYear(), today.getMonth(), today.getDate() - 14))}T14:00:00`,
        series_end: `${dayKey(new Date(today.getFullYear(), today.getMonth(), today.getDate() - 14))}T15:00:00`,
        series_tzid: null,
        rrule: "FREQ=WEEKLY",
        recurrence: { freq: "weekly", interval: 1, by_day: [] },
      }),
      occurrence({
        uid: "quarterly",
        summary: "Quarterly review",
        start: at("12:00:00"),
        end: at("12:30:00"),
        recurring: true,
        recurrence_id: at("12:00:00"),
        series_start: at("12:00:00"),
        series_end: at("12:30:00"),
        rrule: "FREQ=MONTHLY;BYSETPOS=-1;BYDAY=MO,TU",
        recurrence: { freq: "monthly", interval: 1, custom: true },
      }),
      occurrence({
        uid: "offsite",
        summary: "Offsite",
        all_day: true,
        start: dayKey(today),
        end: dayKey(new Date(today.getFullYear(), today.getMonth(), today.getDate() + 1)),
      }),
      occurrence({
        uid: "call",
        summary: "Customer call",
        start: at("16:00:00"),
        end: at("17:00:00"),
        location: "https://us02web.zoom.us/j/123456789?pwd=abc",
      }),
    ],
    next: 1,
    fail: null,
    discoverFails: false,
  };
}

beforeEach(() => {
  native.server = freshServer();
  native.create.mockImplementation(async (calendarId: string, draft: CalendarEventDraftV1) => {
    if (native.server.fail) return err(native.server.fail);
    const uid = `new-${native.server.next++}`;
    native.server.events.push(
      occurrence({
        uid,
        calendar_id: calendarId,
        summary: draft.summary,
        start: draft.start,
        end: draft.end,
        all_day: draft.all_day,
      }),
    );
    return ok({ calendar_id: calendarId, uid });
  });
  native.update.mockImplementation(
    async (calendarId: string, uid: string, draft: CalendarEventDraftV1) => {
      if (native.server.fail) return err(native.server.fail);
      native.server.events = native.server.events.map((entry) =>
        entry.uid === uid
          ? { ...entry, summary: draft.summary, start: draft.start, end: draft.end }
          : entry,
      );
      return ok({ calendar_id: calendarId, uid });
    },
  );
  native.remove.mockImplementation(async (_calendar: string, uid: string) => {
    native.server.events = native.server.events.filter((entry) => entry.uid !== uid);
    return ok(null);
  });
  native.updateOccurrence.mockImplementation(async (calendarId: string, uid: string) =>
    ok({ calendar_id: calendarId, uid }),
  );
  native.removeOccurrence.mockResolvedValue(ok(null));
  native.createTask.mockResolvedValue("task-1");
  native.addAccount.mockImplementation(async () => err("unauthorized"));
  native.open.mockResolvedValue({ accepted: true, operation_id: null });
});

afterEach(() => {
  eventOpen.clear();
  calendar.dispose();
  vi.clearAllMocks();
});

async function setup() {
  await page.viewport(1400, 900);
  const screen = await render(CalendarPage);
  screen.container.style.cssText = "inline-size:1380px;block-size:860px";
  return screen;
}

const eventButton = (screen: Awaited<ReturnType<typeof setup>>, name: RegExp) =>
  screen.getByRole("button", { name }).first();

test("switches between day, week, month and agenda, by button and by key", async () => {
  const screen = await setup();
  await expect.element(eventButton(screen, /Standup/u)).toBeVisible();

  await screen.getByRole("radio", { name: "Month" }).click();
  await expect.element(eventButton(screen, /Standup/u)).toBeVisible();
  expect(screen.container.querySelector(".weeks")).not.toBeNull();

  await screen.getByRole("radio", { name: "Agenda" }).click();
  await expect.element(screen.getByRole("heading", { level: 3 }).first()).toBeVisible();
  expect(screen.container.querySelector(".agenda")).not.toBeNull();

  await page.getByRole("radio", { name: "Day" }).click();
  expect(screen.container.querySelectorAll(".column")).toHaveLength(1);

  // Keys act on the page, not inside a form.
  document.body.focus();
  await userEvent.keyboard("w");
  await vi.waitFor(() => expect(screen.container.querySelectorAll(".column")).toHaveLength(7));
  await userEvent.keyboard("m");
  await vi.waitFor(() => expect(screen.container.querySelector(".weeks")).not.toBeNull());
  await userEvent.keyboard("t");
  await expect.element(eventButton(screen, /Standup/u)).toBeVisible();
});

test("the Today button and arrows move the view", async () => {
  const screen = await setup();
  await expect.element(eventButton(screen, /Standup/u)).toBeVisible();
  await screen.getByRole("button", { name: "Next" }).last().click();
  await expect.element(eventButton(screen, /Standup/u)).not.toBeInTheDocument();
  await screen.getByRole("button", { name: "Today" }).click();
  await expect.element(eventButton(screen, /Standup/u)).toBeVisible();
});

test("a new event is created in the first writable calendar and shown only once saved", async () => {
  const screen = await setup();
  await expect.element(eventButton(screen, /Standup/u)).toBeVisible();
  await screen.getByRole("button", { name: "New event" }).first().click();
  await expect.element(screen.getByRole("form", { name: "New event" })).toBeVisible();

  await screen.getByLabelText("Title").fill("Planning");
  await screen.getByRole("button", { name: "Save" }).click();

  await vi.waitFor(() => expect(native.create).toHaveBeenCalledTimes(1));
  const [calendarId, draft] = native.create.mock.calls[0] as [string, CalendarEventDraftV1];
  expect(calendarId).toBe("work");
  expect(draft).toMatchObject({ summary: "Planning", all_day: false, rrule: null, reminders: [] });
  expect(draft.tzid).toBeTruthy();
  await expect.element(eventButton(screen, /Planning/u)).toBeVisible();
  await expect.element(screen.getByRole("form", { name: "New event" })).not.toBeInTheDocument();
});

test("pressing an empty slot opens a new event at that time", async () => {
  const screen = await setup();
  await expect.element(eventButton(screen, /Standup/u)).toBeVisible();
  const column = screen.container.querySelector<HTMLElement>(".column[data-today]")!;
  // 394 px into a 48 px-per-hour column is 08:12, snapped to 08:15.
  await page.elementLocator(column).click({ position: { x: 20, y: 394 } });
  await expect.element(screen.getByRole("form", { name: "New event" })).toBeVisible();
  await expect.element(screen.getByLabelText("Start time")).toHaveValue("08:15");
  await expect.element(screen.getByLabelText("End time")).toHaveValue("09:15");
});

test("an event is edited and the list follows what native saved", async () => {
  const screen = await setup();
  await eventButton(screen, /Standup/u).click();
  await expect.element(screen.getByRole("heading", { name: "Standup" })).toBeVisible();
  await screen.getByRole("button", { name: "Edit" }).click();

  const title = screen.getByLabelText("Title");
  await expect.element(title).toHaveValue("Standup");
  await title.fill("Daily standup");
  await screen.getByLabelText("End time").fill("11:30");
  await screen.getByRole("button", { name: "Save" }).click();

  await vi.waitFor(() => expect(native.update).toHaveBeenCalledTimes(1));
  const [calendarId, uid, draft] = native.update.mock.calls[0] as [
    string,
    string,
    CalendarEventDraftV1,
  ];
  expect([calendarId, uid, draft.summary]).toEqual(["work", "standup", "Daily standup"]);
  expect(draft.end.endsWith("T11:30:00")).toBe(true);
  await expect.element(eventButton(screen, /Daily standup/u)).toBeVisible();
});

test("a refused save stays in the editor with its reason and shows nothing as saved", async () => {
  native.server.fail = "conflict";
  const screen = await setup();
  await eventButton(screen, /Standup/u).click();
  await screen.getByRole("button", { name: "Edit" }).click();
  await screen.getByLabelText("Title").fill("Changed");
  await screen.getByRole("button", { name: "Save" }).click();

  await expect
    .element(screen.getByRole("alert"))
    .toHaveTextContent(/Someone else changed this event/u);
  await expect.element(screen.getByRole("form", { name: "Edit event" })).toBeVisible();
  expect(native.server.events.find((entry) => entry.uid === "standup")?.summary).toBe("Standup");
});

test("rejects an end before the start without calling native", async () => {
  const screen = await setup();
  await screen.getByRole("button", { name: "New event" }).first().click();
  await screen.getByLabelText("End time").fill("00:00");
  await screen.getByLabelText("Start time").fill("09:00");
  await screen.getByRole("button", { name: "Save" }).click();
  await expect.element(screen.getByText("The end must not be before the start.")).toBeVisible();
  expect(native.create).not.toHaveBeenCalled();
});

test("deleting asks first, then removes the event", async () => {
  const screen = await setup();
  await eventButton(screen, /Customer call/u).click();
  await screen.getByRole("button", { name: "Delete" }).click();
  await expect.element(screen.getByText("Delete this event?")).toBeVisible();
  expect(native.remove).not.toHaveBeenCalled();
  await screen.getByRole("button", { name: "Delete" }).click();
  await vi.waitFor(() => expect(native.remove).toHaveBeenCalledWith("work", "call"));
  await expect.element(eventButton(screen, /Customer call/u)).not.toBeInTheDocument();
});

test("read-only calendars show no edit controls", async () => {
  const screen = await setup();
  await eventButton(screen, /Founders Day/u).click();
  await expect.element(screen.getByRole("heading", { name: "Founders Day" })).toBeVisible();
  await expect.element(screen.getByText("This calendar is read-only.")).toBeVisible();
  await expect.element(screen.getByRole("button", { name: "Edit" })).not.toBeInTheDocument();
  await expect.element(screen.getByRole("button", { name: "Delete" })).not.toBeInTheDocument();
});

test("editing a repeating event asks which events, then writes only that scope", async () => {
  const screen = await setup();
  await eventButton(screen, /Weekly sync/u).click();
  await screen.getByRole("button", { name: "Edit" }).click();
  await expect.element(screen.getByText("Edit which events?")).toBeVisible();
  expect(native.update).not.toHaveBeenCalled();

  await screen.getByRole("button", { name: "This event" }).click();
  await expect.element(screen.getByText("Changes apply to this event only.")).toBeVisible();
  await expect.element(screen.getByLabelText("Every")).not.toBeInTheDocument();
  await screen.getByLabelText("Title").fill("Weekly sync (moved room)");
  await screen.getByRole("button", { name: "Save" }).click();

  await vi.waitFor(() => expect(native.updateOccurrence).toHaveBeenCalledTimes(1));
  const [calendarId, uid, instance, draft] = native.updateOccurrence.mock.calls[0] as [
    string,
    string,
    string,
    CalendarEventDraftV1,
  ];
  expect([calendarId, uid, instance, draft.summary]).toEqual([
    "work",
    "series",
    at("14:00:00"),
    "Weekly sync (moved room)",
  ]);
  expect(native.update).not.toHaveBeenCalled();
});

test("editing all events sends the series start and the structured rule", async () => {
  const screen = await setup();
  await eventButton(screen, /Weekly sync/u).click();
  await screen.getByRole("button", { name: "Edit" }).click();
  await screen.getByRole("button", { name: "All events" }).click();
  await expect.element(screen.getByText("Changes apply to all occurrences.")).toBeVisible();

  const every = screen.getByLabelText("Every");
  await expect.element(every).toHaveValue(1);
  await every.fill("2");
  await screen.getByRole("button", { name: "Save" }).click();

  await vi.waitFor(() => expect(native.update).toHaveBeenCalledTimes(1));
  const [calendarId, uid, draft] = native.update.mock.calls[0] as [
    string,
    string,
    CalendarEventDraftV1,
  ];
  const first = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 14);
  expect([calendarId, uid]).toEqual(["work", "series"]);
  expect(draft.start).toContain(dayKey(first));
  expect(draft.recurrence).toMatchObject({ freq: "weekly", interval: 2 });
  expect(native.updateOccurrence).not.toHaveBeenCalled();
});

test("a rule the form cannot express is shown read-only and sent back untouched", async () => {
  const screen = await setup();
  await eventButton(screen, /Quarterly review/u).click();
  await screen.getByRole("button", { name: "Edit" }).click();
  await screen.getByRole("button", { name: "All events" }).click();
  await expect.element(screen.getByText(/more complex than the form can show/u)).toBeVisible();
  await expect.element(screen.getByText("FREQ=MONTHLY;BYSETPOS=-1;BYDAY=MO,TU")).toBeVisible();
  await expect.element(screen.getByLabelText("Every")).not.toBeInTheDocument();
  await screen.getByRole("button", { name: "Save" }).click();

  await vi.waitFor(() => expect(native.update).toHaveBeenCalledTimes(1));
  const draft = native.update.mock.calls[0]![2] as CalendarEventDraftV1;
  expect(draft.rrule).toBe("FREQ=MONTHLY;BYSETPOS=-1;BYDAY=MO,TU");
  expect(draft.recurrence ?? null).toBeNull();
});

test("deleting a repeating event asks which events and deletes that scope", async () => {
  const screen = await setup();
  await eventButton(screen, /Weekly sync/u).click();
  await screen.getByRole("button", { name: "Delete" }).click();
  await expect.element(screen.getByText("Delete which events?")).toBeVisible();
  expect(native.remove).not.toHaveBeenCalled();
  await screen.getByRole("button", { name: "This event" }).click();
  await vi.waitFor(() =>
    expect(native.removeOccurrence).toHaveBeenCalledWith("work", "series", at("14:00:00")),
  );
  expect(native.remove).not.toHaveBeenCalled();

  await eventButton(screen, /Quarterly review/u).click();
  await screen.getByRole("button", { name: "Delete" }).click();
  await screen.getByRole("button", { name: "All events" }).click();
  await vi.waitFor(() => expect(native.remove).toHaveBeenCalledWith("work", "quarterly"));
});

const neighbour = (screen: Awaited<ReturnType<typeof setup>>, selector: string) => {
  const columns = [...screen.container.querySelectorAll<HTMLElement>(selector)];
  const index = columns.findIndex((column) => column.dataset.day === dayKey(today));
  const target = columns[index < columns.length - 1 ? index + 1 : index - 1]!;
  return { target, key: target.dataset.day! };
};

test("dragging an all-day banner moves it to another day", async () => {
  const screen = await setup();
  const chip = eventButton(screen, /Offsite/u);
  await expect.element(chip).toBeVisible();
  const { target, key } = neighbour(screen, ".banner-column");
  await userEvent.dragAndDrop(chip, page.elementLocator(target));

  await vi.waitFor(() => expect(native.update).toHaveBeenCalledTimes(1));
  const [, uid, draft] = native.update.mock.calls[0] as [string, string, CalendarEventDraftV1];
  expect(uid).toBe("offsite");
  expect(draft).toMatchObject({ all_day: true, start: key });
});

test("dragging a timed event to another day column moves it there", async () => {
  const screen = await setup();
  const block = eventButton(screen, /Customer call/u);
  await expect.element(block).toBeVisible();
  const { target, key } = neighbour(screen, ".column");
  await userEvent.dragAndDrop(block, page.elementLocator(target));

  await vi.waitFor(() => expect(native.update).toHaveBeenCalledTimes(1));
  const [, uid, draft] = native.update.mock.calls[0] as [string, string, CalendarEventDraftV1];
  expect(uid).toBe("call");
  expect(draft.start.startsWith(key)).toBe(true);
});

test("moving a repeating event asks which events first", async () => {
  const screen = await setup();
  const block = eventButton(screen, /Weekly sync/u);
  await expect.element(block).toBeVisible();
  const { target } = neighbour(screen, ".column");
  await userEvent.dragAndDrop(block, page.elementLocator(target));
  await expect.element(screen.getByText("Move which events?")).toBeVisible();
  expect(native.update).not.toHaveBeenCalled();
  expect(native.updateOccurrence).not.toHaveBeenCalled();
  await screen.getByRole("button", { name: "All events" }).click();
  await vi.waitFor(() => expect(native.update).toHaveBeenCalledTimes(1));
  expect(native.update.mock.calls[0]![1]).toBe("series");
});

test("a meeting link opens in a new browser tab", async () => {
  const screen = await setup();
  await eventButton(screen, /Customer call/u).click();
  await screen.getByRole("button", { name: "Join" }).click();
  expect(native.open).toHaveBeenCalledWith("https://us02web.zoom.us/j/123456789?pwd=abc", true);
});

test("hidden calendars come from native, and a change event reloads them", async () => {
  const screen = await setup();
  await expect.element(eventButton(screen, /Standup/u)).toBeVisible();
  native.server.events = native.server.events.filter((entry) => entry.uid !== "standup");
  emitNativeEvent("calendarChanged", { version: 1 });
  await expect.element(eventButton(screen, /Standup/u)).not.toBeInTheDocument();
});

test("the password is never shown back and is cleared when submitted", async () => {
  const screen = await setup();
  await screen.getByRole("button", { name: "Accounts" }).first().click();
  await screen.getByRole("button", { name: "Add CalDAV account" }).click();
  await screen.getByLabelText("Email address or server address").fill("me@example.com");
  await screen.getByRole("button", { name: "Continue" }).click();

  const password = screen.getByLabelText("Password");
  await expect
    .element(screen.getByLabelText("Server address"))
    .toHaveValue("https://cal.example.com/dav");
  await expect.element(password).toHaveAttribute("type", "password");
  await password.fill("s3cret");
  await screen.getByRole("button", { name: "Sign in" }).click();

  await vi.waitFor(() => expect(native.addAccount).toHaveBeenCalledTimes(1));
  expect(native.addAccount).toHaveBeenCalledWith(
    "https://cal.example.com/dav",
    "me@example.com",
    "s3cret",
    null,
  );
  await expect
    .element(screen.getByRole("alert"))
    .toHaveTextContent(/rejected the user name or password/u);
  await expect.element(password).toHaveValue("");
  expect(document.body.innerHTML).not.toContain("s3cret");
});

test("shows a way to add calendars when there are none", async () => {
  native.server.accounts = [];
  native.server.calendars = [];
  native.server.events = [];
  const screen = await setup();
  await expect.element(screen.getByRole("heading", { name: "No calendars yet" })).toBeVisible();
  await expect.element(screen.getByRole("button", { name: "New event" }).first()).toBeDisabled();
});

test("an event becomes a task due when it starts, with a link back to the event", async () => {
  const screen = await setup();
  await eventButton(screen, /Standup/u).click();
  await screen.getByRole("button", { name: "Create task" }).click();

  await expect.element(screen.getByText("Task created")).toBeVisible();
  expect(native.createTask).toHaveBeenCalledTimes(1);
  const input = native.createTask.mock.calls[0]![0] as {
    title: string;
    dueDate: string;
    dueTime: string;
    context: { url: string; title: string };
  };
  expect([input.title, input.dueDate, input.dueTime]).toEqual(["Standup", dayKey(today), "10:00"]);
  expect(parseEventUrl(input.context.url)).toEqual({
    calendarId: "work",
    uid: "standup",
    at: at("10:00:00"),
  });
  // Once made, it is not made again from the same details.
  await expect.element(screen.getByRole("button", { name: "Create task" })).toBeDisabled();
});

test("a task that could not be made says so and can be tried again", async () => {
  native.createTask.mockResolvedValueOnce(null);
  const screen = await setup();
  await eventButton(screen, /Standup/u).click();
  await screen.getByRole("button", { name: "Create task" }).click();
  await expect.element(screen.getByText("The task could not be created.")).toBeVisible();
  await screen.getByRole("button", { name: "Create task" }).click();
  await expect.element(screen.getByText("Task created")).toBeVisible();
  expect(native.createTask).toHaveBeenCalledTimes(2);
});

test("opens on the event another surface asked for, on its own day", async () => {
  const later = new Date(today.getFullYear(), today.getMonth(), today.getDate() + 21);
  const start = `${dayKey(later)}T15:00:00`;
  native.server.events = [
    occurrence({
      uid: "far",
      summary: "Offsite dinner",
      start,
      end: `${dayKey(later)}T17:00:00`,
    }),
  ];
  eventOpen.open(linkOf(native.server.events[0]!));
  const screen = await setup();
  await expect.element(screen.getByRole("heading", { name: "Offsite dinner" })).toBeVisible();
  expect(eventOpen.pending()).toBeNull();
});

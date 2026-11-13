import { beforeAll, describe, expect, it } from "vitest";
import type { CalendarOccurrenceV1 } from "../../../shared/ipc/bindings";
import { place } from "../dates";
import {
  editPolicy,
  formForSlot,
  formFromEvent,
  formToDraft,
  movedForm,
  noRecurrence,
  wallClock,
} from "../draft";
import type { EventForm } from "../draft";
import { meetingLink } from "../meeting";

beforeAll(() => {
  process.env.TZ = "Europe/Berlin";
});

const base: EventForm = {
  calendarId: "cal",
  summary: "  Review  ",
  allDay: false,
  startDate: "2026-10-07",
  startTime: "09:00",
  endDate: "2026-10-07",
  endTime: "10:30",
  location: " Room 4 ",
  description: "Notes",
  recurrence: noRecurrence(),
  reminder: "15",
  url: null,
  moreReminders: [],
  zone: null,
};

const occurrence = (patch: Partial<CalendarOccurrenceV1> = {}): CalendarOccurrenceV1 => ({
  calendar_id: "cal",
  uid: "u1",
  recurrence_id: null,
  summary: "Review",
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
  ...patch,
});

describe("event drafts", () => {
  it("writes timed events as wall-clock times in the zone", () => {
    expect(formToDraft(base, "Europe/Berlin")).toEqual({
      ok: true,
      draft: {
        summary: "Review",
        description: "Notes",
        location: "Room 4",
        start: "2026-10-07T09:00:00",
        end: "2026-10-07T10:30:00",
        all_day: false,
        tzid: "Europe/Berlin",
        rrule: null,
        recurrence: null,
        reminders: [15],
        url: null,
      },
    });
  });

  it("keeps the event's own zone and its further reminders", () => {
    const result = formToDraft(
      { ...base, zone: "America/New_York", moreReminders: [60] },
      "Europe/Berlin",
    );
    expect(result.ok && [result.draft.start, result.draft.tzid, result.draft.reminders]).toEqual([
      "2026-10-07T03:00:00",
      "America/New_York",
      [15, 60],
    ]);
    const odd = formToDraft({ ...base, zone: "Custom/Nowhere" }, "Europe/Berlin");
    expect(odd.ok && odd.draft.tzid).toBe("Europe/Berlin");
  });

  it("converts to another zone", () => {
    const result = formToDraft(base, "America/New_York");
    expect(result.ok && result.draft.start).toBe("2026-10-07T03:00:00");
  });

  it("keeps the wall clock on the daylight-saving day", () => {
    expect(wallClock(new Date(2026, 2, 29, 3, 30), "Europe/Berlin")).toBe("2026-03-29T03:30:00");
  });

  it("writes an all-day event with an exclusive end", () => {
    const result = formToDraft({ ...base, allDay: true, endDate: "2026-10-09" }, "Europe/Berlin");
    expect(result.ok && [result.draft.start, result.draft.end, result.draft.tzid]).toEqual([
      "2026-10-07",
      "2026-10-10",
      null,
    ]);
  });

  it("sends a structured recurrence and no RRULE", () => {
    const send = (patch: Partial<EventForm["recurrence"]>) => {
      const result = formToDraft({ ...base, recurrence: { ...noRecurrence(), ...patch } });
      return result.ok ? [result.draft.recurrence, result.draft.rrule] : result.error;
    };
    expect(send({})).toEqual([null, null]);
    expect(send({ freq: "daily" })).toEqual([{ freq: "daily", interval: 1 }, null]);
    expect(send({ freq: "weekly", interval: "2", byDay: ["MO", "WE"] })).toEqual([
      { freq: "weekly", interval: 2, by_day: ["MO", "WE"] },
      null,
    ]);
    expect(send({ freq: "monthly", ends: "count", count: "5" })).toEqual([
      { freq: "monthly", interval: 1, count: 5 },
      null,
    ]);
    expect(send({ freq: "yearly", ends: "until", until: "2027-01-01" })).toEqual([
      { freq: "yearly", interval: 1, until: "2027-01-01" },
      null,
    ]);
  });

  it("refuses an invalid interval, count or end date", () => {
    const send = (patch: Partial<EventForm["recurrence"]>) =>
      formToDraft({ ...base, recurrence: { ...noRecurrence(), freq: "daily", ...patch } });
    const refused = { ok: false, error: "recurrence" };
    expect(send({ interval: "0" })).toEqual(refused);
    expect(send({ interval: "1.5" })).toEqual(refused);
    expect(send({ interval: "1001" })).toEqual(refused);
    expect(send({ ends: "count", count: "" })).toEqual(refused);
    expect(send({ ends: "until", until: "" })).toEqual(refused);
    expect(send({ ends: "until", until: "2026-10-01" })).toEqual(refused);
  });

  it("keeps a custom rule through its RRULE and the day selectors of a kept frequency", () => {
    const custom = formToDraft({
      ...base,
      recurrence: {
        ...noRecurrence(),
        freq: "custom",
        rrule: "FREQ=MONTHLY;BYSETPOS=-1;BYDAY=MO,TU",
      },
    });
    expect(custom.ok && [custom.draft.recurrence, custom.draft.rrule]).toEqual([
      null,
      "FREQ=MONTHLY;BYSETPOS=-1;BYDAY=MO,TU",
    ]);
    const keep = { freq: "monthly", by_day: ["2TU"], by_month_day: [], by_month: [] };
    const same = formToDraft({
      ...base,
      recurrence: { ...noRecurrence(), freq: "monthly", keep },
    });
    expect(same.ok && same.draft.recurrence).toEqual({
      freq: "monthly",
      interval: 1,
      by_day: ["2TU"],
      by_month_day: [],
      by_month: [],
    });
    const changed = formToDraft({
      ...base,
      recurrence: { ...noRecurrence(), freq: "yearly", keep },
    });
    expect(changed.ok && changed.draft.recurrence).toEqual({ freq: "yearly", interval: 1 });
  });

  it("refuses an end before the start and a bad reminder", () => {
    expect(formToDraft({ ...base, endTime: "08:00" })).toEqual({ ok: false, error: "range" });
    expect(formToDraft({ ...base, startDate: "2026-02-31" })).toEqual({
      ok: false,
      error: "range",
    });
    expect(formToDraft({ ...base, allDay: true, endDate: "2026-10-06" })).toEqual({
      ok: false,
      error: "range",
    });
    expect(formToDraft({ ...base, reminder: "-5" })).toEqual({ ok: false, error: "reminder" });
    expect(formToDraft({ ...base, reminder: "1.5" })).toEqual({ ok: false, error: "reminder" });
    expect(formToDraft({ ...base, reminder: "" })).toMatchObject({
      ok: true,
      draft: { reminders: [] },
    });
  });

  it("round-trips an existing all-day event with an inclusive end date", () => {
    const event = place(occurrence({ all_day: true, start: "2026-10-07", end: "2026-10-09" }))!;
    const form = formFromEvent(event);
    expect([form.startDate, form.endDate]).toEqual(["2026-10-07", "2026-10-08"]);
    const result = formToDraft(form);
    expect(result.ok && result.draft.end).toBe("2026-10-09");
  });

  it("builds a slot form and moves an event keeping its length", () => {
    const slot = formForSlot("cal", new Date(2026, 9, 7, 14), new Date(2026, 9, 7, 15));
    expect([slot.startTime, slot.endTime, slot.allDay]).toEqual(["14:00", "15:00", false]);
    const event = place(occurrence({ start: "2026-10-07T23:00:00", end: "2026-10-08T00:30:00" }))!;
    const moved = movedForm(event, new Date(2026, 9, 9, 8, 15));
    expect([moved.startDate, moved.startTime, moved.endDate, moved.endTime]).toEqual([
      "2026-10-09",
      "08:15",
      "2026-10-09",
      "09:45",
    ]);
    const allDay = place(occurrence({ all_day: true, start: "2026-10-07", end: "2026-10-09" }))!;
    const shifted = movedForm(allDay, new Date(2026, 9, 10));
    expect([shifted.startDate, shifted.endDate]).toEqual(["2026-10-10", "2026-10-11"]);
  });
});

const series = (patch: Partial<CalendarOccurrenceV1> = {}) =>
  place(
    occurrence({
      recurring: true,
      recurrence_id: "2026-10-14T09:00:00",
      start: "2026-10-14T09:00:00",
      end: "2026-10-14T10:00:00",
      series_start: "2026-10-07T09:00:00",
      series_end: "2026-10-07T10:00:00",
      series_tzid: "Europe/Berlin",
      rrule: "FREQ=WEEKLY;BYDAY=WE;COUNT=8",
      recurrence: { freq: "weekly", interval: 1, count: 8, by_day: ["WE"] },
      ...patch,
    }),
  )!;

describe("repeating events", () => {
  it("edits the series from its own start and rule, or the occurrence from its own times", () => {
    const all = formFromEvent(series(), "all");
    expect([all.startDate, all.startTime, all.endTime, all.zone]).toEqual([
      "2026-10-07",
      "09:00",
      "10:00",
      "Europe/Berlin",
    ]);
    expect(all.recurrence).toMatchObject({
      freq: "weekly",
      interval: "1",
      ends: "count",
      count: "8",
      byDay: ["WE"],
    });
    const sent = formToDraft(all, "Europe/Berlin");
    expect(sent.ok && [sent.draft.start, sent.draft.recurrence]).toEqual([
      "2026-10-07T09:00:00",
      { freq: "weekly", interval: 1, count: 8, by_day: ["WE"] },
    ]);
    const one = formFromEvent(series(), "this");
    expect([one.startDate, one.recurrence.freq]).toEqual(["2026-10-14", "none"]);
  });

  it("shows a rule the form cannot express as custom, with its RRULE", () => {
    const form = formFromEvent(
      series({
        rrule: "FREQ=MONTHLY;BYSETPOS=-1;BYDAY=MO,TU",
        recurrence: { freq: "monthly", interval: 1, custom: true },
      }),
      "all",
    );
    expect(form.recurrence).toMatchObject({
      freq: "custom",
      rrule: "FREQ=MONTHLY;BYSETPOS=-1;BYDAY=MO,TU",
    });
  });

  it("moves the series by the distance the occurrence moved", () => {
    const moved = movedForm(series(), new Date(2026, 9, 15, 11, 30), "all");
    expect([moved.startDate, moved.startTime, moved.endDate, moved.endTime]).toEqual([
      "2026-10-08",
      "11:30",
      "2026-10-08",
      "12:30",
    ]);
    const one = movedForm(series(), new Date(2026, 9, 15, 11, 30), "this");
    expect([one.startDate, one.startTime]).toEqual(["2026-10-15", "11:30"]);
  });

  it("moves an all-day series by days only", () => {
    const allDay = series({
      all_day: true,
      start: "2026-10-14",
      end: "2026-10-15",
      series_start: "2026-10-07",
      series_end: "2026-10-08",
      series_tzid: null,
    });
    const moved = movedForm(allDay, new Date(2026, 9, 16), "all");
    expect([moved.startDate, moved.endDate]).toEqual(["2026-10-09", "2026-10-09"]);
  });

  it("only treats read-only calendars as not editable", () => {
    expect(editPolicy(occurrence())).toBe("edit");
    expect(editPolicy(occurrence({ recurring: true }))).toBe("edit");
    expect(editPolicy(occurrence({ read_only: true, recurring: true }))).toBe("read-only");
  });
});

describe("meeting links", () => {
  const link = (patch: Partial<CalendarOccurrenceV1>) => meetingLink(occurrence(patch));

  it("finds Teams, Meet and Zoom links in the url, location or notes", () => {
    expect(link({ url: "https://teams.microsoft.com/l/meetup-join/19%3a123/0?x=1" })?.service).toBe(
      "teams",
    );
    expect(link({ location: "https://meet.google.com/abc-defg-hij" })?.service).toBe("meet");
    expect(link({ description: "Join: https://us02web.zoom.us/j/12345?pwd=abc." })?.url).toBe(
      "https://us02web.zoom.us/j/12345?pwd=abc",
    );
  });

  it("ignores other addresses, lookalike hosts and insecure links", () => {
    expect(link({ location: "https://example.com/j/1" })).toBeNull();
    expect(link({ location: "https://zoom.us.evil.example/j/1" })).toBeNull();
    expect(link({ location: "http://meet.google.com/abc-defg-hij" })).toBeNull();
    expect(link({ location: "https://teams.microsoft.com/" })).toBeNull();
    expect(link({})).toBeNull();
  });
});
